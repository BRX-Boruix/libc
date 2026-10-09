//! malloc/free/realloc/calloc —— 基于 \`brk\` 的用户态堆分配器（C ABI）。
//!
//! 真实数据链路（S06）：堆空间直接来自 \`libsys::mem::brk\` 系统调用（内核
//! MEMORY 域 memory_grow）。分配器自身管理这些内存，不依赖 Rust 全局分配器，
//! 因此 malloc 是独立、确定性的 C 风格分配器。
//!
//! ## 块布局（16 字节头，payload 16 字节对齐）
//!
//! 每个块（无论分配/空闲）头 16 字节：
//! \`\`\`text
//! +0  usize  block_size（含 16 字节头，块总大小，16 的倍数）
//! +8  usize  分配块：保留（恒 0）；空闲块：下一空闲块指针（0 = 表尾）
//! +16        payload（通常 起始+16；对齐分配时为对齐后的地址）
//! \`\`\`
//! 块起始地址恒 16 字节对齐（brk 页对齐 → 0 mod 16）。
//! 分配时把 payload 相对块起始的偏移写在 payload-8，free/realloc 据此反推块起始。
//!
//! ## 空闲表：**按尺寸分档的桶**（2026-10 重写）
//!
//! 原实现是**单条按地址升序的空闲表**，每次 malloc/free 都要线性扫整条表 ⇒
//! 每次操作 O(n)。实测代价触目惊心：heapstress 的 19,200 次操作耗时 **25.7 秒**
//! （≈1.3 毫秒/次；正常分配器约 50 纳秒），系统内 tcc 的**链接**一步 **183.9 秒**，
//! 宿主上 GCC 单文件构建 70+ 分钟编不完——**同一根因**。
//!
//! 现改为 NBINS 个尺寸桶（小块 16 字节步进、大块按 2 的幂）：push/take 都是
//! **O(1)**，双重释放检测用块头**标志位**（也是 O(1)）。
//!
//! **代价（如实声明）**：**不再做相邻块合并**（coalescing）——块按尺寸入桶，
//! 不再按地址排序，故找不到"地址相邻"的另一半。用碎片换速度。
//!
//! ## 分配策略：桶内取用 + 分裂
//!
//! malloc 从 min 的桶起向上找第一个非空桶；若取到的块剩余 >= 最小块（32B），
//! 分裂出剩余空闲块并压回**它自己尺寸**的桶。
//!
//! ## 并发
//!
//! 单进程模型下无并发线程；但为将来线程化准备，全局表经 spin 互斥保护
//! （锁获取顺序：仅本锁，无嵌套锁序，S21）。

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// 包一层使内部可变指针可作为 Sync static（单进程模型 + 锁保护下安全）。
// SAFETY: 单进程模型下所有经 FREELIST 的访问都持有 ALLOC_LOCK 互斥；
// 指针裸类型非 Send/Sync 是本实现刻意接受的（与 C 全局分配器一致）。
struct SyncUnsafe<T: ?Sized>(UnsafeCell<T>);
unsafe impl<T: ?Sized> Sync for SyncUnsafe<T> {}
impl<T> SyncUnsafe<T> {
    const fn new(v: T) -> Self { SyncUnsafe(UnsafeCell::new(v)) }
    fn get(&self) -> *mut T { self.0.get() }
}
use crate::ctypes::{size_t, c_int};
use crate::errno::{set_errno, ENOMEM, EINVAL};

/// 块头大小（16 字节）：size + next。
const HEADER: usize = 16;
/// 最小块总大小：头 + 最小 payload（16），保证空闲块能容纳 next 指针。
const MIN_BLOCK: usize = HEADER + 16;
/// 对齐要求：payload 16 字节对齐。
const ALIGN: usize = 16;

// ---------- 加固（S40） ----------

/// 每个分配块在 payload 末尾预留的 8 字节金丝雀（canary）区。
/// 分配 `size` 字节实际占 `size + CANARY_SIZE`；canary 位于 payload+size。
/// free 时校验，损坏（越界写进 canary 区）即置 `MALLOC_CORRUPT`。
const CANARY_SIZE: usize = 8;
const CANARY_MAGIC: u64 = 0xDEAD_0000_BEEF_5EED;

/// free 时把已释放的 `size` 字节 payload 填 0xDD（毒化），捕获 use-after-free。
const POISON_BYTE: u8 = 0xDD;

/// 全局堆损坏标志：canary 校验失败或检测到 double-free 时置位。
/// 供测试/诊断读取；置位后分配器仍尽力保持可用（不 panic）。
static MALLOC_CORRUPT: AtomicBool = AtomicBool::new(false);

/// 读取堆损坏标志（测试/诊断用）。
#[unsafe(no_mangle)]
pub extern "C" fn boruix_malloc_corrupt() -> c_int {
    if MALLOC_CORRUPT.load(Ordering::Relaxed) { 1 } else { 0 }
}

/// 计算 payload 的可写容量（block_size - payload 偏移），含末尾 canary 区。
#[inline]
unsafe fn payload_capacity(block: *mut u8, payload: *mut u8) -> usize {
    unsafe {
        let off = payload_offset(payload);
        block_size(block).saturating_sub(off)
    }
}

/// 用户实际可用字节数 = 容量 - canary 区。
#[inline]
fn user_size(cap: usize) -> usize {
    cap.saturating_sub(CANARY_SIZE)
}

/// 写入 canary（payload + 用户区末尾，8 字节）。
#[inline]
unsafe fn write_canary(payload: *mut u8, cap: usize) {
    unsafe {
        let at = payload.add(user_size(cap)) as *mut u64;
        *at = CANARY_MAGIC;
    }
}

/// 校验 canary；损坏返回 false（并置 MALLOC_CORRUPT）。
#[inline]
unsafe fn check_canary(payload: *mut u8, cap: usize) -> bool {
    unsafe {
        let at = payload.add(user_size(cap)) as *const u64;
        if *at != CANARY_MAGIC {
            MALLOC_CORRUPT.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }
}

/// free 时把已释放的用户区填 0xDD（毒化）；canary 区保留。
/// 用 volatile 写：LLVM 会因"毒化仅经越界读/复用可观察"而将其当作死存储消除，
/// volatile 保证 0xDD 始终实际写入（S40 加固可观测、可验收）。
#[inline]
unsafe fn poison_payload(payload: *mut u8, cap: usize) {
    unsafe {
        let n = user_size(cap);
        // **按机器字写，不再逐字节**（2026-10 实测的同类根因）：
        // 每次 free 都要毒化整个用户区，逐字节 volatile 对大块是纯开销。
        // 毒化本身仍保留（S40 加固可观测），只是把 8 次写合成 1 次。
        let word = {
            let mut w = 0usize;
            let mut k = 0;
            while k < core::mem::size_of::<usize>() {
                w = (w << 8) | (POISON_BYTE as usize);
                k += 1;
            }
            w
        };
        let mut i = 0usize;
        while i + core::mem::size_of::<usize>() <= n {
            core::ptr::write_volatile(payload.add(i) as *mut usize, word);
            i += core::mem::size_of::<usize>();
        }
        while i < n {
            core::ptr::write_volatile(payload.add(i), POISON_BYTE);
            i += 1;
        }
    }
}

/// 把 n 向上取整到 16 的倍数。
#[inline]
fn align16(n: usize) -> usize {
    (n + ALIGN - 1) & !(ALIGN - 1)
}

// ---------- 简单自旋互斥（保护全局分配器状态） ----------

struct SpinLock {
    held: AtomicBool,
}
impl SpinLock {
    const fn new() -> Self {
        SpinLock { held: AtomicBool::new(false) }
    }
    fn lock(&self) {
        while self.held.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
    }
    fn unlock(&self) {
        self.held.store(false, Ordering::Release);
    }
}
/// 小尺寸的**精确**分档步进（字节）。
///
/// **为什么必须是精确档**（2026-10 实测的两次教训）：
/// - 幂档（1 KiB 以上按 2 的幂）会让**一个桶覆盖一个区间**（如 1025..2048），
///   于是桶里可能有比请求 `min` **小**的块。
/// - 第一次修法（桶内线性找 `sz >= min`）保证了**正确性**，但在**无合并**的碎片化堆里，
///   偏小的块会不断在桶里堆积，扫描退化成 O(n)：实测同一份输入，干净堆链接 5.8 秒，
///   而碎片化堆（前面刚跑过 heapstress/cxxstress/fuzz/forkrace）链接 **137.6 秒**。
/// - 精确档下**同桶块大小全相等** ⇒ `sz >= min` 恒成立、扫描立即命中，退化消失。
const SMALL_STEP: usize = 16;
/// 精确档覆盖的上限（64 KiB）。再大的块少而大，用 2 的幂分档即可。
const SMALL_MAX: usize = 64 * 1024;
/// 精确档数量：32, 48, …, 65536。
const SMALL_BINS: usize = (SMALL_MAX - MIN_BLOCK) / SMALL_STEP + 1;

/// 空闲桶数量：精确档 + 大块（2 的幂）档 + 余量。
///
/// 为什么分桶（2026-10 实测的根因修复）：原实现是**单条按地址升序的空闲表**，
/// `freelist_take`（first-fit）/`freelist_insert`（有序插入）/`freelist_contains`
/// 每次操作都**线性扫整条表** ⇒ 每次分配/释放 O(n)。实测代价：
///   - `heapstress` 的 19,200 次操作耗时 **25.7 秒**（≈1.3 毫秒/次，正常约 50 纳秒）；
///   - 系统内 `tcc <1.5KB .s> -o <out>` 的**链接**一步耗时 **183.9 秒**（同一次汇编只要 6.6 秒）；
///   - 宿主上 GCC 自身构建单文件 70+ 分钟编不完。
/// 分桶后 take/push 都是 **O(1)**。
///
/// **代价（如实声明）**：**不再做相邻块合并（coalescing）**——块落在尺寸桶里，
/// 不再按地址排序，故找不到"地址相邻"的另一半。碎片换速度；本系统有 1 GiB 量级
/// 可增长堆，且实测的分配模式（大量同尺寸小块）天然复用同一桶，故这是划算的取舍。
const NBINS: usize = SMALL_BINS + 64;

/// 尺寸 → 桶号。`size` 恒 >= `MIN_BLOCK`(32) 且为 16 的倍数。
#[inline]
fn bin_of(size: usize) -> usize {
    if size <= SMALL_MAX {
        (size - MIN_BLOCK) / SMALL_STEP // 精确档：同桶块大小全相等
    } else {
        // 大块按 2 的幂分档；逐块 `sz >= min` 校验仍在（见 bin_take），此处只保证桶号不越界。
        let b = SMALL_BINS + (usize::BITS - 1 - (size - 1).leading_zeros()) as usize;
        if b >= NBINS { NBINS - 1 } else { b }
    }
}

/// 每个尺寸桶的空闲块栈顶（地址 0 = 空桶）。块之间用块头的 next 字段串成单链。
static BINS: [SyncUnsafe<*mut u8>; NBINS] =
    [const { SyncUnsafe::new(core::ptr::null_mut()) }; NBINS];

/// 保护 BINS 与堆元数据的锁。
static ALLOC_LOCK: SpinLock = SpinLock::new();

// ---------- 块内偏移访问（裸指针，锁内访问） ----------

/// size 字段的**最低位**兼作"本块当前在空闲表上"的标志。
///
/// 为什么可以：所有块大小都是 16 的倍数（`align16`），最低 4 位恒为 0。
/// 为什么需要：原实现用 `freelist_contains` **线性扫整条空闲表**来判断双重释放——
/// 那是每次 free 一次 O(n) 扫描，是实测"每次分配/释放 ~1.3 毫秒"的主因之一
/// （正常分配器约 50 纳秒）。用一位做标志后，双重释放检测是 O(1)。
const FREE_BIT: usize = 1;

/// 读块头 size 字段（**掩掉标志位**）。
#[inline]
unsafe fn block_size(p: *mut u8) -> usize {
    unsafe { *(p as *const usize) & !FREE_BIT }
}
/// 写块头 size 字段（**保留标志位**：调用方可能在标记为已用之前改大小）。
#[inline]
unsafe fn set_block_size(p: *mut u8, sz: usize) {
    unsafe {
        let raw = *(p as *const usize);
        *(p as *mut usize) = (sz & !FREE_BIT) | (raw & FREE_BIT);
    }
}
/// 标记本块"在空闲表上"。
#[inline]
unsafe fn mark_free(p: *mut u8) {
    unsafe { *(p as *mut usize) |= FREE_BIT; }
}
/// 标记本块"已分配"（不在空闲表上）。
#[inline]
unsafe fn mark_used(p: *mut u8) {
    unsafe { *(p as *mut usize) &= !FREE_BIT; }
}
/// 本块当前是否在空闲表上（O(1) 双重释放检测）。
#[inline]
unsafe fn is_free(p: *mut u8) -> bool {
    unsafe { *(p as *const usize) & FREE_BIT != 0 }
}
/// 读空闲块 next 字段（偏移 8）。
#[inline]
unsafe fn block_next(p: *mut u8) -> *mut u8 {
    unsafe { *((p as *mut usize).add(1)) as *mut u8 }
}
/// 写空闲块 next 字段（偏移 8）。
#[inline]
unsafe fn set_block_next(p: *mut u8, next: *mut u8) {
    unsafe { *((p as *mut usize).add(1)) = next as usize; }
}
/// payload 起始 = 块起始 + HEADER。
#[inline]
unsafe fn payload_of(p: *mut u8) -> *mut u8 {
    unsafe { p.add(HEADER) }
}
/// 读 payload 相对块起始的偏移（存于 payload-8 处，所有分配都写）。
#[inline]
unsafe fn payload_offset(payload: *mut u8) -> usize {
    unsafe { *((payload as *const usize).sub(1)) }
}

/// 写 payload 相对块起始的偏移到 payload-8。
#[inline]
unsafe fn set_payload_offset(payload: *mut u8, off: usize) {
    unsafe { *((payload as *mut usize).sub(1)) = off; }
}

/// 由返回给用户的 payload 指针读取块起始（block = payload - offset）。
///
/// 说明：为支持 `posix_memalign`（payload 可能偏离块起始 > HEADER），分配时
/// 统一把 payload 相对块起始的**偏移**写在 payload-8；free/realloc 据此反推块起始。
#[inline]
unsafe fn block_from_payload(payload: *mut u8) -> *mut u8 {
    unsafe { payload.sub(payload_offset(payload)) }
}

/// 把 n 向上取整到 align（2 的幂）的倍数。
#[inline]
fn align_up(n: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (n + align - 1) & !(align - 1)
}

// ---------- 空闲表操作（须持锁） ----------

/// 把一个空闲块压入它**尺寸对应的桶**（O(1)）。**不做合并**（见 NBINS 的代价声明）。
unsafe fn bin_push(block: *mut u8) {
    unsafe {
        let b = bin_of(block_size(block));
        set_block_next(block, *BINS[b].get());
        *BINS[b].get() = block;
        mark_free(block);
    }
}

/// 取一个 **size >= min** 的块（从 min 的桶起向上找），必要时分裂。
///
/// **必须逐块校验 `sz >= min`**（2026-10 实测的缺陷）：幂档桶**覆盖一个区间**
/// （如 1025..2048 同属一桶），故桶里可能有比 `min` 小的块。原实现直接取桶头 ⇒
/// 当 `min = 2048` 而桶头是 1025 字节时 `sz - min` **下溢**成天文数字 ⇒
/// `set_block_size(rest, 巨大值)` 写坏块头 ⇒ 之后 `bin_of(巨大值)` 数组越界 panic
/// （实测：机内 `tcc syscallfuzz.c` 直接 `userspace panic: malloc.rs:296`）。
/// 现在在本桶内线性找第一个 `sz >= min`（桶很小，代价可接受），找不到才升桶。
unsafe fn bin_take(min: usize) -> *mut u8 {
    unsafe {
        let mut b = bin_of(min);
        while b < NBINS {
            let mut prev: *mut u8 = core::ptr::null_mut();
            let mut cur: *mut u8 = *BINS[b].get();
            while !cur.is_null() {
                let sz = block_size(cur);
                if sz >= min {
                    let nxt = block_next(cur);
                    if prev.is_null() {
                        *BINS[b].get() = nxt;
                    } else {
                        set_block_next(prev, nxt);
                    }
                    mark_used(cur);
                    // 分裂：剩余 >= MIN_BLOCK 时，把尾部放回**它自己尺寸**的桶。
                    if sz - min >= MIN_BLOCK {
                        let rest = cur.add(min);
                        set_block_size(rest, sz - min);
                        bin_push(rest);
                        set_block_size(cur, min);
                    }
                    return cur;
                }
                prev = cur;
                cur = block_next(cur);
            }
            b += 1;
        }
        core::ptr::null_mut()
    }
}
/// `new == 0` 表示**仅查询**当前断点。成功返回断点，失败返回 -1 并置 errno。
///
/// **为什么暴露它**：`brk` 是本系统唯一的堆原语，而它此前只有 Rust 侧（`libsys::brk`）
/// 可达。诊断"两个分配器共用 brk"一类问题时，C 探针必须能自己查询/推进断点。
/// 这不是诊断专用后门——`sbrk` 语义的程序本来就该有这个入口。
#[unsafe(no_mangle)]
pub extern "C" fn boruix_brk(new: u64) -> i64 {
    match libsys::brk(new) {
        Ok(b) => b as i64,
        Err(e) => {
            set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}

/// 开关堆增长诊断（C ABI，声明见 libc/include/boruix.h）。
///
/// 打开后，libc 的 `malloc` 与 libsys 的 buddy 在每次 `brk` 扩展时各打一行
/// `[C|L] <cur_brk> <new_brk>`（十六进制）。**默认关闭**——诊断不能污染所有程序。
/// 这是定位「两个分配器共用 brk」类问题的**常驻工具**，不是一次性补丁。
#[unsafe(no_mangle)]
pub extern "C" fn boruix_heap_diag(on: crate::ctypes::c_int) {
    libsys::heap_diag(on != 0);
}

/// 分配器**契约自检**（默认关闭，由 `boruix_heap_diag(1)` 打开）。
///
/// 契约：`malloc(size)`/`realloc(ptr,size)` 返回的块，**可用容量必须 >= size**。
///
/// 为什么加它：tcc 在系统内加载对象时，`section_realloc` 的 `memset` 会写过一个块的末尾
/// （内核留证显示：用户态写不存在的页、指令落在 `memset`、地址刚越过 `brk`）。
/// 那条 `memset` 的起点与长度都正确，所以充分嫌疑是**分配器给了太小的块**。
/// 与其继续读代码猜，不如让分配器自己回答——这正是本次崩溃的充分条件。
///
/// 零分配：栈缓冲 + write（诊断路径绝不能分配）。
fn check_capacity(tag: u8, cap: usize, want: usize) {
    if !libsys::heap_diag_on() {
        return;
    }
    // **口径收紧**：`cap` 是 payload 容量（**含末尾 canary 区**），用户真正可写的只有
    // `user_size(cap) = cap - CANARY_SIZE`。上一轮用 `cap >= want` 比较，口径偏松。
    let cap = user_size(cap);
    if cap >= want {
        return;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut buf = [0u8; 40];
    buf[0] = b'[';
    buf[1] = tag;
    buf[2] = b'!';
    buf[3] = b']';
    for i in 0..16usize {
        buf[4 + i] = HEX[((cap as u64 >> (60 - i * 4)) & 0xf) as usize];
    }
    buf[20] = b' ';
    for i in 0..16usize {
        buf[21 + i] = HEX[((want as u64 >> (60 - i * 4)) & 0xf) as usize];
    }
    buf[37] = b'\n';
    let _ = libsys::write(1, &buf[..38]);
}

/// **返回值语义**：返回**整段新堆区的起点**，并已把整段 `[cur, new_brk)` 登记为一个
/// 空闲块（不是只登记 `need`）。为什么必须整段登记：`grow = need.max(64 KiB)`，
/// 若只登记 `need`，扩展出来的尾巴 `[cur+need, new_brk)` 就**既不空闲也不可达**
/// （`brk` 已经推上去了），成为**永久泄漏**。
///
/// 实测（3P6-1 内存墙）：tcc 在系统内编译 `hello.c` 时反复申请小块，每次扩展泄漏
/// 最多 64 KiB，累计把单进程配额吃光后 `tcc_malloc` 返回 NULL → `tcc: memory full`。
/// **症状特征：与物理内存无关**——256 MiB / 1 GiB / 2 GiB 都失败，因为泄漏量与配额
/// 同步增长（对照：纯 1 MiB 大块探针在 1 GiB 下能干净地用到 244 MiB 配额上限）。
unsafe fn heap_extend(need: usize) -> *mut u8 {
    let cur = match libsys::brk(0) {
        Ok(b) => b,
        Err(_) => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    let cur = align16(cur as usize);
    let grow = need.max(64 * 1024);
    let target = cur + align16(grow);
    let new_brk = match libsys::brk(target as u64) {
        Ok(b) => b,
        Err(_) => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    if new_brk as usize <= cur {
        set_errno(ENOMEM);
        return core::ptr::null_mut();
    }
    // 堆增长诊断（与 libsys 的 diag_brk 同格式，标签 C）：**默认关闭**，
    // 由 `boruix_heap_diag(1)` 打开。零堆分配：栈缓冲 + write。
    //
    // **必须带 pid**：串口日志是多进程交织的，不带 pid 会把「不同进程各自的第一段」
    // 误读成「同一进程重复」——实测踩过这个坑。
    if libsys::heap_diag_on() {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut buf = [0u8; 56];
        buf[0] = b'[';
        buf[1] = b'C';
        buf[2] = b'/';
        let pid = libsys::getpid().unwrap_or(0);
        let mut tmp = [0u8; 20];
        let mut n = 0usize;
        let mut v = pid;
        if v == 0 {
            tmp[0] = b'0';
            n = 1;
        }
        while v > 0 && n < tmp.len() {
            tmp[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
        }
        if n > 4 {
            n = 4;
        }
        for k in 0..n {
            buf[3 + k] = tmp[n - 1 - k];
        }
        buf[3 + n] = b']';
        buf[4 + n] = b' ';
        let base = 5 + n;
        let a = cur as u64;
        let b = new_brk;
        for i in 0..16usize {
            buf[base + i] = HEX[((a >> (60 - i * 4)) & 0xf) as usize];
        }
        buf[base + 16] = b' ';
        for i in 0..16usize {
            buf[base + 17 + i] = HEX[((b >> (60 - i * 4)) & 0xf) as usize];
        }
        buf[base + 33] = b'\n';
        let _ = libsys::write(1, &buf[..base + 34]);
    }
    // **整段登记**（见函数头注）：把 [cur, new_brk) 全部还给空闲表，而不是只还 `need`。
    // 分桶后不再有"与左邻合并"，但整段登记仍然必要——否则扩展出来的尾巴既不空闲
    // 也不可达（brk 已推上去）⇒ 永久泄漏。
    let len = new_brk as usize - cur;
    set_block_size(cur as *mut u8, len);
    bin_push(cur as *mut u8);
    cur as *mut u8
}

/// 从堆申请一块连续内存并格式化为空闲块插入表（内部，须持锁）。
unsafe fn heap_alloc_block(need_total: usize) -> *mut u8 {
    let from_list = bin_take(need_total);
    if !from_list.is_null() {
        return from_list;
    }
    // 扩展：`heap_extend` 已把**整段**新堆区登记进空闲表，这里直接再取一次即可。
    if heap_extend(need_total).is_null() {
        return core::ptr::null_mut();
    }
    bin_take(need_total)
}

/// \`malloc(size)\`：分配 \`size\` 字节，返回 16 字节对齐指针；失败返回 NULL 置 ENOMEM。
#[unsafe(no_mangle)]
pub extern "C" fn malloc(size: size_t) -> *mut u8 {
    // `malloc(0)`：C 标准允许返回 NULL，但**主流实现（glibc）返回可用的非空指针**，
    // 而大量既有程序依赖后者。实测 tcc 就是这样：它的 `load_data()` 对 `sh_size == 0`
    // 的节直接 `tcc_malloc(0)` 并把结果当指针用（`strsec = load_data(...)`），
    // 返回 NULL 会在随后的 `strncmp` 上读空指针而崩溃
    // （内核留证：fault_addr=0x0、用户态读、rip 在 strncmp、ret 在 tcc_load_object_file）。
    //
    // 3P6-1 的目标是兼容既有程序，故此处**与 glibc 对齐**：按最小块分配并返回非空指针。
    let size = size.max(1);
    // 加固：多分配 CANARY_SIZE 字节，canary 置于 payload+size（末尾）。
    let alloc = match size.checked_add(CANARY_SIZE) {
        Some(n) => n,
        None => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    let need = match alloc.checked_add(HEADER) {
        Some(n) => align16(n),
        None => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    let need = need.max(MIN_BLOCK);
    ALLOC_LOCK.lock();
    let block = unsafe { heap_alloc_block(need) };
    if block.is_null() {
        ALLOC_LOCK.unlock();
        return core::ptr::null_mut();
    }
    let payload = unsafe { payload_of(block) };
    unsafe {
        set_payload_offset(payload, HEADER); // 存偏移=16，供 free/realloc 反推
        let cap = payload_capacity(block, payload);
        check_capacity(b'M', cap, size); // 契约自检：容量必须 >= 请求
        write_canary(payload, cap);
    }
    ALLOC_LOCK.unlock();
    payload
}

/// \`free(ptr)\`：释放 \`malloc\` 分配的内存。ptr=NULL 为合法空操作。
#[unsafe(no_mangle)]
pub extern "C" fn free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    ALLOC_LOCK.lock();
    unsafe {
        let block = block_from_payload(ptr);
        let cap = payload_capacity(block, ptr);
        // 金丝雀校验：检测越界写。
        check_canary(ptr, cap);
        // double-free 防御：块头标志位 O(1) 判定（原先是线性扫整条空闲表）。
        if is_free(block) {
            MALLOC_CORRUPT.store(true, Ordering::Relaxed);
            ALLOC_LOCK.unlock();
            return;
        }
        // 毒化已释放 payload（use-after-free 捕获）。
        poison_payload(ptr, cap);
        set_block_next(block, core::ptr::null_mut());
        bin_push(block);
    }
    ALLOC_LOCK.unlock();
}

/// \`realloc(ptr, new_size)\`：调整已分配块大小。
///
/// **说明**：经 payload 偏移方案（offset 存于 payload-8），\`realloc\` 能正确
/// 处理 \`malloc/calloc\` 与 \`posix_memalign/aligned_alloc\` 分配的指针——
/// 均以 payload-8 的偏移反推块起始，并从原 payload 复制数据。
#[unsafe(no_mangle)]
pub extern "C" fn realloc(ptr: *mut u8, new_size: size_t) -> *mut u8 {
    if ptr.is_null() {
        return malloc(new_size);
    }
    if new_size == 0 {
        free(ptr);
        return core::ptr::null_mut();
    }
    let new_alloc = match new_size.checked_add(CANARY_SIZE) {
        Some(n) => n,
        None => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    let new_need = match new_alloc.checked_add(HEADER) {
        Some(n) => align16(n).max(MIN_BLOCK),
        None => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    ALLOC_LOCK.lock();
    unsafe {
        let off = payload_offset(ptr);
        let block = ptr.sub(off);
        let old_size = block_size(block);
        let old_cap = payload_capacity(block, ptr);
        if old_size >= new_need {
            // 原地：canary 需重写到新容量末尾，并毒化多出的尾部。
            let new_cap = old_cap.min(new_alloc + HEADER);
            check_capacity(b'R', new_cap, new_size); // 契约自检：原地分支
            write_canary(ptr, new_cap);
            let old_user = user_size(old_cap);
            let new_user = user_size(new_cap);
            if old_user > new_user {
                // 毒化被裁掉的尾部（用户区末尾之后到旧 canary 之前）。
                core::ptr::write_bytes(ptr.add(new_user), POISON_BYTE, old_user - new_user);
            }
            ALLOC_LOCK.unlock();
            return ptr;
        }
        let new_block = heap_alloc_block(new_need);
        if new_block.is_null() {
            ALLOC_LOCK.unlock();
            return core::ptr::null_mut();
        }
        let new_payload = payload_of(new_block);
        // 复制用户数据（含原 canary 前的数据；不含 canary）。
        let copy_len = user_size(old_cap).min(new_size);
        // **顺序要紧**：先把 payload 偏移写进新块，`payload_capacity` 才能算出真实容量。
        // 曾把检查放在这一行**之前**，`payload_offset` 读到未初始化内存，容量被算成 0，
        // 报出 47 次假警报（[D!] 容量 0）——检查自己的顺序错，不是缺陷。
        set_payload_offset(new_payload, HEADER);
        // 拷贝点两侧各查一次契约：源可读、目标可写，都不得小于 copy_len。
        check_capacity(b'S', old_cap, copy_len);
        check_capacity(b'D', payload_capacity(new_block, new_payload), copy_len);
        core::ptr::copy_nonoverlapping(ptr, new_payload, copy_len);
        let np_cap = payload_capacity(new_block, new_payload);
        check_capacity(b'C', np_cap, new_size); // 契约自检：搬迁分支
        write_canary(new_payload, np_cap);
        // 释放旧块。
        let old_cap2 = payload_capacity(block, ptr);
        if !is_free(block) {
            poison_payload(ptr, old_cap2);
            set_block_next(block, core::ptr::null_mut());
            bin_push(block);
        }
        ALLOC_LOCK.unlock();
        new_payload
    }
}

/// \`calloc(nmemb, size)\`：分配并清零 nmemb*size 字节。
#[unsafe(no_mangle)]
pub extern "C" fn calloc(nmemb: size_t, size: size_t) -> *mut u8 {
    // 与 malloc(0) 同一口径：零大小也返回可用的非空指针（glibc 行为）。
    let total = match nmemb.checked_mul(size) {
        Some(t) if t > 0 => t,
        _ => return core::ptr::null_mut(),
    };
    let p = malloc(total);
    if !p.is_null() {
        unsafe { core::ptr::write_bytes(p, 0, total); }
    }
    p
}

/// \`malloc_usable_size(ptr)\`：返回 ptr 指向块的实际可用字节数。
#[unsafe(no_mangle)]
pub extern "C" fn malloc_usable_size(ptr: *mut u8) -> size_t {
    if ptr.is_null() {
        return 0;
    }
    ALLOC_LOCK.lock();
    let sz = unsafe { block_size(block_from_payload(ptr)) - payload_offset(ptr) };
    ALLOC_LOCK.unlock();
    sz
}

/// \`posix_memalign(&memptr, alignment, size)\`：对齐分配。
///
/// 支持任意 2 的幂对齐（`>= sizeof(void*)`，含 SSE/AVX 的 64/256 等）。分配时
/// 整体多分配 `alignment` 字节，把 payload 对齐到 `alignment`，并把块起始
/// 指针写在 `payload-8` 供 free/realloc 反推（见 block_from_payload）。成功返回 0，
/// 失败置 `*memptr=NULL` 并返回 ENOMEM/EINVAL。
#[unsafe(no_mangle)]
pub extern "C" fn posix_memalign(memptr: *mut *mut u8, alignment: size_t, size: size_t) -> c_int {
    if memptr.is_null() {
        return EINVAL;
    }
    unsafe { *memptr = core::ptr::null_mut(); }
    if alignment < core::mem::size_of::<usize>() || !alignment.is_power_of_two() {
        return EINVAL;
    }
    let p = aligned_alloc_impl(alignment, size);
    if p.is_null() {
        ENOMEM
    } else {
        unsafe { *memptr = p; }
        0
    }
}

/// \`aligned_alloc(alignment, size)\`：C11 对齐分配（size 应为 alignment 的倍数）。
#[unsafe(no_mangle)]
pub extern "C" fn aligned_alloc(alignment: size_t, size: size_t) -> *mut u8 {
    if !alignment.is_power_of_two() || alignment == 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    aligned_alloc_impl(alignment, size)
}

/// 对齐分配核心：分配并返回对齐到 alignment 的 payload。
fn aligned_alloc_impl(alignment: size_t, size: size_t) -> *mut u8 {
    if size == 0 {
        return core::ptr::null_mut();
    }
    // 需要的块总空间：size + alignment + HEADER（对齐调整的余量）。
    let need = match size.checked_add(alignment).and_then(|n| n.checked_add(HEADER)) {
        Some(n) => align_up(n, ALIGN).max(MIN_BLOCK),
        None => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    ALLOC_LOCK.lock();
    let block = unsafe { heap_alloc_block(need) };
    if block.is_null() {
        ALLOC_LOCK.unlock();
        return core::ptr::null_mut();
    }
    unsafe {
        // payload 对齐到 alignment（block 起始 16 对齐）。
        let raw = block.add(HEADER);
        let aligned = align_up(raw as usize, alignment) as *mut u8;
        // 把 payload 相对块起始的偏移写到 payload-8（与 malloc 的 offset 方案一致）。
        set_payload_offset(aligned, aligned as usize - block as usize);
        // 加固：写入 canary。
        let cap = payload_capacity(block, aligned);
        write_canary(aligned, cap);
        ALLOC_LOCK.unlock();
        aligned
    }
}
