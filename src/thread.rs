//! 线程引导 / 每线程控制块（TCB）与用户态 FS 段基址（threads.md T2-1/T2-2，ADR-035 D6）。
//!
//! 每个线程在共享地址空间持一个独立 mmap 的 `Tcb`，errno 槽位于 **offset 0**；线程引导把
//! x86-64 `IA32_FS_BASE`（MSR 0xC0000100）设为该 `Tcb` 地址。**RDMSR/WRMSR 是 CPL0 特权指令，
//! 用户态直用会 #GP**，故写侧经内核 `set_fs_base` syscall（threads.md T2-1，kernel 于 CPL0 内
//! wrmsr），读侧用 `rdfsbase`（FSGSBASE 指令，内核已置 CR4.FSGSBASE=1，见 kernel c699d99）。
//! 内核调度器对每线程保存/恢复 FS base（kernel 4f0b724，threads.md T2-0），保证线程切换后
//! FS base 恒指向当前线程 Tcb。
//!
//! 装配方（`write_fs_base`/`install_tcb`）通常由线程入口（经 libsys `thread_spawn_with_starter` 的
//! a3 starter 引导）首条调用；组长/主线程入口也须装配自己的 `Tcb`，否则其 errno 回落进程级兜底槽。

/// 每线程控制块（TCB）。`errno` 必须在 **offset 0**（`__errno_location` 直接把 FS base
/// 当 errno 槽地址返回，故首字段即 errno）。其余字段 T2-1 阶段保留；T2-2 在其上加 TLS 区。
#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub struct Tcb {
    /// errno 槽（offset 0；FS base 即指向此，`FS:[0]` 读写即 errno）。
    pub errno: i32,
    _pad: i32,
    /// 本线程 pid（调试/gettid 缓存，可后续由 pthread_self 用之）。
    pub tid: u64,
    /// 组长进程 pid（tgid）。
    pub tgid: u64,
    /// 自指针（可选自校验）。
    pub this: u64,
    // ---- T2-2 库管理 TLS 区（bump arena，仅本线程读写，无锁）----
    /// 本线程 TLS arena 基址（线程私有 mmap 区，可与本 Tcb 异址）。
    pub tls_base: u64,
    /// 下一次分配地址（bump 游标；arena 内单调前进；单写者本线程）。
    pub tls_cursor: u64,
    /// arena 结束地址（排他上界）；cursor + size > end 则拒绝（S09 越界拒接）。
    pub tls_end: u64,
}

impl Tcb {
    /// 全零 Tcb（errno=0，id 与 TLS arena 待填）。
    pub const fn zeroed() -> Self {
        Self { errno: 0, _pad: 0, tid: 0, tgid: 0, this: 0, tls_base: 0, tls_cursor: 0, tls_end: 0 }
    }
}

/// 读当前线程 FS 段基址：`rdfsbase`（FSGSBASE 指令，需内核已置 CR4.FSGSBASE=1，见 kernel
/// `cpu::enable_fsgsbase`）。不读 MSR（RDMSR 是 CPL0 特权指令，用户态会 #GP）。
/// 未装配（引导未设 FS base=0）时返回 0。
#[inline]
pub fn read_fs_base() -> u64 {
    let mut base: u64;
    unsafe {
        core::arch::asm!("rdfsbase {b}", b = out(reg) base, options(nostack, preserves_flags));
    }
    base
}

/// 写当前线程 FS 段基址为 `base`。线程引导装配 `Tcb` 用：置 FS base = `Tcb` 地址。
/// WRMSR 是 CPL0 特权指令（用户态会 #GP），故经内核 `set_fs_base` syscall（CPL0 内 wrmsr）写；
/// 内核已对每线程保存/恢复 FS base（kernel 4f0b724 T2-0），本调用后下次切出即自动归档。
#[inline]
pub fn write_fs_base(base: u64) {
    // 失败（如内核不支持）静默：装配失败时 FS base 保持 0，errno 回落进程兜底槽，可观测不崩溃。
    let _ = libsys::set_fs_base(base);
}

/// 便捷：装配本线程 `Tcb`（设 FS base 指向它）并回填 `tid`/`tgid`/`this`。
/// 线程引导在首条用户指令调用：先 mmap/零初始化一个 `Tcb`，把地址传本函数。
#[inline]
pub fn install_tcb(tcb: &mut Tcb, tid: u64, tgid: u64) {
    tcb.tid = tid;
    tcb.tgid = tgid;
    tcb.this = tcb as *mut Tcb as u64;
    write_fs_base(tcb as *mut Tcb as u64);
}

/// 取当前线程 errno 槽的可写指针（供 `errno` 模块 / `__errno_location` 使用）：
/// FS base 非零 → 其指向 `Tcb`（errno 在 offset 0）；零 → `None`（调用方回落进程兜底槽）。
#[inline]
pub fn current_errno_ptr_or_null() -> *mut i32 {
    let base = read_fs_base();
    if base != 0 { base as *mut i32 } else { core::ptr::null_mut() }
}

// ---- T2-2 库管理 TLS 区（threads.md T2-2，S09：库管理仿真，非编译器 `__thread`）----

/// 把 `tcb` 的 TLS arena 装配为 `[base, base+len)`、游标清零。通常由组长/引导在派生前对该线程的
/// `Tcb` 调用（`tcb` 是其线程私有 mmap 的每线程控制块地址），使该线程装配 FS base 后即可
/// `tls_allocate`。`len` 为 arena 字节数。
#[inline]
pub fn tls_setup(tcb: *mut Tcb, base: u64, len: u64) {
    unsafe {
        (*tcb).tls_base = base;
        (*tcb).tls_cursor = base;
        (*tcb).tls_end = base.wrapping_add(len);
    }
}

/// 当前线程 TLS bump 分配 `size` 字节、按 `align` 对齐，返回 arena 内指针；不足/未装配 FS base
/// 或 arena → `None`。只在本线程的 arena（经 FS base→其 `Tcb.tls_*`）分配，故同调用在多个线程
/// 各得各自私有块，互不干扰（threads.md T2-2 每线程 TLS 独立）。生命周期 = 线程（无 per-slot free，
/// S09 声明；线程退出 arena mmap 随进程/线程释放）。对齐须为 2 幂。
#[inline]
pub fn tls_allocate(size: usize, align: usize) -> Option<*mut u8> {
    let base = read_fs_base();
    if base == 0 { return None; } // 未装配 Tcb/FS base
    debug_assert!(align.is_power_of_two());
    let tcb = base as *mut Tcb;
    // arena 装配检查：base/end 一致才视为就绪（cursor 在 [base,end)）。
    let (ab, cur, end) = unsafe { ((*tcb).tls_base, (*tcb).tls_cursor, (*tcb).tls_end) };
    if end <= ab || cur < ab || cur >= end { return None; }
    let align = align.max(1);
    let mask = (align as u64).wrapping_sub(1);
    let aligned = (cur.wrapping_add(mask)) & !mask;
    let need = size as u64;
    if aligned.checked_add(need).map_or(true, |a| a > end) { return None; } // S09 越界拒接
    unsafe { (*tcb).tls_cursor = aligned.wrapping_add(need); }
    Some(aligned as *mut u8)
}
