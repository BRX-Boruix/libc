//! errno 机制与错误码常量（与内核 ADR-010 错误码对齐，S13 常量单点）。
//!
//! C 标准要求 `errno` 是一个线程相关的可写整型，程序出错后读取以获知具体
//! 错误。
//!
//! ## 进程局部性（B5，S09 如实声明）
//!
//! 本内核采用**进程隔离内存**（每个用户进程有独立地址空间），故每个进程的
//! `ERRNO` 静态量天然**进程局部**——A 进程写 errno 不影响 B 进程。内核当前为
//! **单进程无线程**模型（ADR-003 纯 spawn，无线程），进程内也只有一个执行流，
//! 因此用单个全局原子整型即可正确表达语义。
//!
//! ## 未来线程化的缺口
//!
//! 若未来内核支持**进程内多线程**（pthread），进程内多个线程会共享同一地址
//! 空间、共享此 `ERRNO` 静态量，届时 errno 将变为线程共享而非线程局部，违反
//! C 语义。该情况下须把 `ERRNO` 改为**线程局部存储（TLS）**：经
//! `__errno_location()` 返回每线程的地址。当前实现已在 `__errno_location`
//! 处预留了"按指针读写"的接口形态，便于未来切换到 TLS 而无需改动调用方。
//!
//! 约定（S09 宁缺毋假）：
//! - 库函数失败时**显式设置** errno（由 `set_errno` 记录）；成功不保证
//!   不改变 errno（C 标准允许）。
//!
//! 错误码数值与内核 `klib::error::Error::to_errno` 对齐（ADR-010）：
//! 见 libsys `error.rs` 的 `Error::to_errno` 映射。

use core::sync::atomic::{AtomicI32, Ordering};

/// 全局 errno 值（当前单进程模型下即"当前线程"的 errno）。
static ERRNO: AtomicI32 = AtomicI32::new(0);

/// 设置 errno 为 `e`。
#[inline]
pub fn set_errno(e: i32) {
    ERRNO.store(e, Ordering::Relaxed);
}

/// 读取当前 errno 值。
#[inline]
pub fn errno() -> i32 {
    ERRNO.load(Ordering::Relaxed)
}

/// 供按指针读写 errno 的代码使用（`errno` 宏 / `strerror` 内部）。
/// 单进程模型下返回全局 ERRNO 的地址；返回的指针在进程生命周期内稳定。
#[unsafe(no_mangle)]
pub extern "C" fn __errno_location() -> *mut i32 {
    // ERRNO 为 'static AtomicI32，按 C 约定单线程读写其内部值。
    unsafe { &mut *(core::ptr::addr_of!(ERRNO) as *mut AtomicI32 as *mut i32) }
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
        libsys::Error::NoSpace => ENOSPC,
        libsys::Error::Io => EIO,
        libsys::Error::Unknown(n) => n,
    }
}
