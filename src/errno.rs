//! errno 机制与错误码常量（与内核 ADR-010 错误码对齐，S13 常量单点）。
//!
//! C 标准要求 `errno` 是一个线程相关的可写整型，程序出错后读取以获知具体
//! 错误。
//!
//! ## 进程局部性 + 线程局部性（threads.md T2-1，ADR-035 D6，S09 如实声明）
//!
//! BORUIX 采用**进程隔离内存**（每进程独立地址空间），故进程级静态天然进程局部。
//! 同进程内多线程（threads.md T1 已完成：同组共享地址空间的独立调度单元）会共享本进程
//! 地址空间，须让 errno **线程局部**才符合 C 语义。落地（见
//! `docs/DESIGN-T2-TLS-errno-threadlocal.md`，kernel 4f0b724 T2-0）：
//!
//! - 每线程持一个用户态 `Tcb`（`crate::thread::Tcb`，独立 mmap，errno 槽在 **offset 8**——
//!   3P4-1 引入编译器 TLS 后 offset 0 被 x86-64 TLS ABI 的"线程指针自指针"占用，见该结构文档）；
//!   线程引导把 `IA32_FS_BASE`（CPL3 可写）设为该 `Tcb` 地址；内核调度器对每线程
//!   保存/恢复 FS base（T2-0），保证切换后 FS base 恒指向当前线程 `Tcb`。
//! - `__errno_location()`/`errno()`/`set_errno()` 经 `rdmsr` 读当前线程 FS base：非零 → 其指向
//!   的 `Tcb.errno`（每线程独立）；零（本线程未装配 `Tcb`）→ 退回进程级 `FALLBACK_ERRNO` 槽
//!   ——对确实未装配线程化的进程（单执行上下文）语义正确。
//! - **诚实边界**：errno 是线程局部的充分条件是"该线程已装配 FS base→其 `Tcb`"。一个进程内
//!   凡未装配的线程（如旧式未经 libc 线程引导派生的裸线程）共享 `FALLBACK_ERRNO`。
//!   库层只在装配了 FS base 的线程上保证严格 per-thread（配合 `crate::thread::install_tcb` 使用）。
//!
//! 约定（S09 宁缺毋假）：
//! - 库函数失败时**显式设置** errno（由 `set_errno` 记录）；成功不保证
//!   不改变 errno（C 标准允许）。
//!
//! 错误码数值与内核 `klib::error::Error::to_errno` 对齐（ADR-010）：
//! 见 libsys `error.rs` 的 `Error::to_errno` 映射。

/// 进程级兜底 errno 槽：仅当当前线程 FS base == 0（未装配 `Tcb`，即单执行上下文或
/// 未经 libc 线程引导派生的裸线程）时使用。见模块文档诚实边界。
static FALLBACK_ERRNO: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(0);

/// 返回当前线程 errno 槽的可写指针：FS base 非零（本线程已装配 `Tcb`）→ `Tcb.errno`
/// （offset 8）；零 → 进程级兜底槽。槽在存活期地址稳定。
#[inline]
fn current_errno_ptr() -> *mut i32 {
    let slot = crate::thread::current_errno_ptr_or_null();
    if slot.is_null() {
        unsafe { &mut *(core::ptr::addr_of!(FALLBACK_ERRNO) as *mut core::sync::atomic::AtomicI32 as *mut i32) }
    } else {
        slot
    }
}

/// 设置当前线程 errno 为 `e`。
#[inline]
pub fn set_errno(e: i32) {
    unsafe { *current_errno_ptr() = e; }
}

/// 读取当前线程 errno 值。
#[inline]
pub fn errno() -> i32 {
    unsafe { *current_errno_ptr() }
}

/// 供按指针读写 errno 的代码使用（`errno` 宏 / `strerror` 内部）。
/// 返回当前线程 errno 槽地址（per-thread `Tcb.errno` 或进程兜底槽）。
#[unsafe(no_mangle)]
pub extern "C" fn __errno_location() -> *mut i32 {
    current_errno_ptr()
}

// ---------- 错误码常量（S13：与 libsys::Error::to_errno 对齐） ----------

/// 参数非法 EINVAL。
pub const EINVAL: i32 = 22;
/// 数值越界 ERANGE。
pub const ERANGE: i32 = 34;
/// 目标不存在 ENOENT。
pub const ENOENT: i32 = 2;
/// 目标已存在 EEXIST。
pub const EEXIST: i32 = 17;
/// 不支持的操作 ENOTSUP。
pub const ENOTSUP: i32 = 95;
/// 非阻塞操作无法立即完成 EAGAIN。
pub const EAGAIN: i32 = 11;
/// 阻塞中的系统调用被信号打断 EINTR（ADR-051）。
pub const EINTR: i32 = 4;
/// 空间不足 ENOSPC。
pub const ENOSPC: i32 = 28;
/// 设备 I/O 错误 EIO。
pub const EIO: i32 = 5;
/// 内存耗尽 ENOMEM。
pub const ENOMEM: i32 = 12;
/// 无效文件描述符 EBADF。
pub const EBADF: i32 = 9;
/// 文件描述符表满 EMFILE。
pub const EMFILE: i32 = 24;
/// 参数列表太长 / 超出范围 E2BIG。
pub const E2BIG: i32 = 7;
/// 打开的文件过多 ENFILE。
pub const ENFILE: i32 = 23;
/// 目录非空 ENOTEMPTY。
pub const ENOTEMPTY: i32 = 39;
/// 不是目录 ENOTDIR。
pub const ENOTDIR: i32 = 20;
/// 是目录 EISDIR。
pub const EISDIR: i32 = 21;
/// `EILSEQ`：非法字节序列（宽字符转换失败）。
pub const EILSEQ: i32 = 84;
/// 可执行格式非法 ENOEXEC。
pub const ENOEXEC: i32 = 8;
/// 权限不足 EACCES。
pub const EACCES: i32 = 13;
/// 地址非法 EFAULT。
pub const EFAULT: i32 = 14;
/// 名称过长 ENAMETOOLONG。
pub const ENAMETOOLONG: i32 = 36;
/// 符号链接层数过多 ELOOP。
pub const ELOOP: i32 = 40;
/// 非法 seek ESPIPE。
pub const ESPIPE: i32 = 29;
/// 只读文件系统 EROFS。
pub const EROFS: i32 = 30;
/// 资源忙 EBUSY。
pub const EBUSY: i32 = 16;
/// 文件系统结构损坏 EUCLEAN。
pub const EUCLEAN: i32 = 117;

/// 把 libsys 的 `Error` 映射为 C errno 数值。
///
/// 这是 errno 的**唯一真实来源**（S06）：库函数捕获 libsys 返回的错误，
/// 经本函数转成 C errno 写入 `ERRNO`。禁止任何库函数自行编造 errno。
#[inline]
pub fn from_libsys(e: libsys::Error) -> i32 {
    match e {
        libsys::Error::OutOfMemory => ENOMEM,
        libsys::Error::InvalidParam => EINVAL,
        libsys::Error::OutOfRange => ERANGE,
        libsys::Error::NotFound => ENOENT,
        libsys::Error::AlreadyExists => EEXIST,
        libsys::Error::NotSupported => ENOTSUP,
        libsys::Error::WouldBlock => EAGAIN,
    libsys::Error::Interrupted => EINTR,
        libsys::Error::NoSpace => ENOSPC,
        libsys::Error::Io => EIO,
        libsys::Error::NotDirectory => ENOTDIR,
        libsys::Error::IsDirectory => EISDIR,
        libsys::Error::PermissionDenied => EACCES,
        libsys::Error::BadAddress => EFAULT,
        libsys::Error::NotEmpty => ENOTEMPTY,
        libsys::Error::NameTooLong => ENAMETOOLONG,
        libsys::Error::ArgListTooLong => E2BIG,
        libsys::Error::TooManySymlinks => ELOOP,
        libsys::Error::IllegalSeek => ESPIPE,
        libsys::Error::ExecFormat => ENOEXEC,
        libsys::Error::ReadOnly => EROFS,
        libsys::Error::Corrupt => EUCLEAN,
        libsys::Error::Busy => EBUSY,
        libsys::Error::Unknown(n) => n,
    }
}
