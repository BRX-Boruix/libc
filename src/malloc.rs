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
//! ## 空闲表
//!
//! 按地址升序的单向链表（\`FREELIST\`）。按地址排序使相邻块合并（coalescing）
//! 只需检查链表中的直接邻居即可——插入时维护有序，free 时合并左右邻。
//!
//! ## 分配策略：first-fit + 分裂
//!
//! malloc 从表头线性查找第一个足够大的块；若剩余 >= 最小块（32B），分裂出
//! 剩余空闲块并回插。释放时按地址插入并尝试与邻居合并。
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

/// 全局空闲块链表的表头指针（地址 0 = 空表）。
static FREELIST: SyncUnsafe<*mut u8> = SyncUnsafe::new(core::ptr::null_mut());
/// 保护 FREELIST 与堆元数据的锁。
static ALLOC_LOCK: SpinLock = SpinLock::new();

// ---------- 块内偏移访问（裸指针，锁内访问） ----------

/// 读块头 size 字段。
#[inline]
unsafe fn block_size(p: *mut u8) -> usize {
    unsafe { *(p as *const usize) }
}
/// 写块头 size 字段。
#[inline]
unsafe fn set_block_size(p: *mut u8, sz: usize) {
    unsafe { *(p as *mut usize) = sz; }
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

/// 把 \`block\`（其 size 已正确设置）插入空闲表，按地址升序，并尝试合并邻居。
unsafe fn freelist_insert(block: *mut u8) {
    unsafe {
        let start = block as usize;
        let end = start + block_size(block);
        let mut prev: *mut u8 = core::ptr::null_mut();
        let mut cur: *mut u8 = *FREELIST.get();
        while !cur.is_null() && (cur as usize) < start {
            prev = cur;
            cur = block_next(cur);
        }
        // 与右邻合并：若本块 end == cur start。
        let merged = block;
        if !cur.is_null() && end == (cur as usize) {
            let new_size = block_size(merged) + block_size(cur);
            set_block_size(merged, new_size);
            set_block_next(merged, block_next(cur));
        } else {
            set_block_next(merged, cur);
        }
        // 与左邻合并：若 prev 的 end == merged start。
        if !prev.is_null() {
            let prev_end = (prev as usize) + block_size(prev);
            if prev_end == (merged as usize) {
                let new_size = block_size(prev) + block_size(merged);
                set_block_size(prev, new_size);
                set_block_next(prev, block_next(merged));
                return;
            }
        }
        if prev.is_null() {
            *FREELIST.get() = merged;
        } else {
            set_block_next(prev, merged);
        }
    }
}

/// 从空闲表取出第一个 size >= \`min\` 的块（first-fit），必要时分裂。
/// 返回块起始指针；无足够块返回 null。
unsafe fn freelist_take(min: usize) -> *mut u8 {
    unsafe {
        let mut prev: *mut u8 = core::ptr::null_mut();
        let mut cur: *mut u8 = *FREELIST.get();
        while !cur.is_null() {
            let sz = block_size(cur);
            if sz >= min {
                let nxt = block_next(cur);
                if prev.is_null() {
                    *FREELIST.get() = nxt;
                } else {
                    set_block_next(prev, nxt);
                }
                // 分裂：若剩余 >= MIN_BLOCK，分出尾部空闲块。
                if sz - min >= MIN_BLOCK {
                    let rest = cur.add(min);
                    set_block_size(rest, sz - min);
                    freelist_insert(rest);
                    set_block_size(cur, min);
                }
                return cur;
            }
            prev = cur;
            cur = block_next(cur);
        }
        core::ptr::null_mut()
    }
}

/// 向内核申请至少 \`need\` 字节的新堆区（brk），返回新区起始地址。
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
    cur as *mut u8
}

/// 从堆申请一块连续内存并格式化为空闲块插入表（内部，须持锁）。
unsafe fn heap_alloc_block(need_total: usize) -> *mut u8 {
    let from_list = freelist_take(need_total);
    if !from_list.is_null() {
        return from_list;
    }
    let base = heap_extend(need_total);
    if base.is_null() {
        return core::ptr::null_mut();
    }
    set_block_size(base, need_total);
    freelist_insert(base);
    freelist_take(need_total)
}

/// \`malloc(size)\`：分配 \`size\` 字节，返回 16 字节对齐指针；失败返回 NULL 置 ENOMEM。
#[unsafe(no_mangle)]
pub extern "C" fn malloc(size: size_t) -> *mut u8 {
    if size == 0 {
        return core::ptr::null_mut();
    }
    let need = match size.checked_add(HEADER) {
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
    unsafe { set_payload_offset(payload, HEADER); } // 存偏移=16，供 free/realloc 反推
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
        set_block_next(block, core::ptr::null_mut());
        freelist_insert(block);
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
    let new_need = match new_size.checked_add(HEADER) {
        Some(n) => align16(n).max(MIN_BLOCK),
        None => { set_errno(ENOMEM); return core::ptr::null_mut(); }
    };
    ALLOC_LOCK.lock();
    unsafe {
        let off = payload_offset(ptr);
        let block = ptr.sub(off);
        let old_size = block_size(block);
        if old_size >= new_need {
            ALLOC_LOCK.unlock();
            return ptr;
        }
        let new_block = heap_alloc_block(new_need);
        if new_block.is_null() {
            ALLOC_LOCK.unlock();
            return core::ptr::null_mut();
        }
        let new_payload = payload_of(new_block);
        let copy_len = (old_size - off).min(new_size);
        core::ptr::copy_nonoverlapping(ptr, new_payload, copy_len);
        set_payload_offset(new_payload, HEADER);
        set_block_next(block, core::ptr::null_mut());
        freelist_insert(block);
        ALLOC_LOCK.unlock();
        new_payload
    }
}

/// \`calloc(nmemb, size)\`：分配并清零 nmemb*size 字节。
#[unsafe(no_mangle)]
pub extern "C" fn calloc(nmemb: size_t, size: size_t) -> *mut u8 {
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
        ALLOC_LOCK.unlock();
        aligned
    }
}
