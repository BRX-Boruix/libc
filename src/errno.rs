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
/// 功能未实现 ENOSYS（POSIX；用于「本系统没有该能力」的**如实**拒绝，
/// 例如 `posix_spawnattr_setflags` 的进程组/信号屏蔽/调度优先级旗标）。
pub const ENOSYS: i32 = 38;
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
/// 操作不允许 EPERM。
///
/// **诚实边界**：内核只有**一个** `PermissionDenied`，本 libc 把它映射到 `EACCES`（文件访问
/// 语义，见 `from_libsys`）。故**内核错误永远不会产生 `EPERM`**——本常量存在是为了让 C 程序
/// 能比较（头文件一直有 `#define EPERM`，而 Rust 侧此前没有，两侧不对称，由
/// `libc/tools/audit_errno_sync.py` 列出）。
pub const EPERM: i32 = 1;
/// 资源忙 EBUSY。
pub const EBUSY: i32 = 16;
/// 文件系统结构损坏 EUCLEAN。
pub const EUCLEAN: i32 = 117;

// ---------- 完整 POSIX/Linux errno 符号表（S13：与 include/errno.h 一一对应） ----------
//
// **为什么要有这一整张表**：`errno.h` 按 POSIX 必须定义**完整的符号集**，而不是"本内核会
// 产生的那几个"。第三方代码会直接比较这些名字——真实触发（3P6-3，libstdc++ 的
// `<system_error>` / `std::errc`）：缺 `EDEADLK` 一个名字就报 60 次
// `'EDEADLK' was not declared in this scope`，整批约 1300 条错误。
//
// **诚实边界（S09）**：`Error::to_errno`（libsys）只会产生上面那 23 个具名映射 + `Unknown(n)`，
// 即下面这些常量里**绝大多数本内核永远不会返回**。定义它们是为了让 C/C++ 程序能编译与比较，
// **不是**声称本系统会产生这些错误。数值取 Linux x86_64 的标准编号（与上面既有常量同一套）。
/// 无此进程 ESRCH。
pub const ESRCH: i32 = 3;
/// 无此设备或地址 ENXIO。
pub const ENXIO: i32 = 6;
/// 无子进程 ECHILD。
pub const ECHILD: i32 = 10;
/// 需要块设备 ENOTBLK。
pub const ENOTBLK: i32 = 15;
/// 跨设备链接 EXDEV。
pub const EXDEV: i32 = 18;
/// 无此设备 ENODEV。
pub const ENODEV: i32 = 19;
/// 不适当的 ioctl ENOTTY。
pub const ENOTTY: i32 = 25;
/// 文本文件忙 ETXTBSY。
pub const ETXTBSY: i32 = 26;
/// 文件过大 EFBIG。
pub const EFBIG: i32 = 27;
/// 链接数过多 EMLINK。
pub const EMLINK: i32 = 31;
/// 管道破裂 EPIPE。
pub const EPIPE: i32 = 32;
/// 数学参数超出定义域 EDOM。
pub const EDOM: i32 = 33;
/// 资源死锁 EDEADLK。
pub const EDEADLK: i32 = 35;
/// 无可用锁 ENOLCK。
pub const ENOLCK: i32 = 37;
/// 无期望类型的消息 ENOMSG。
pub const ENOMSG: i32 = 42;
/// 标识符已删除 EIDRM。
pub const EIDRM: i32 = 43;
/// 通道号超范围 ECHRNG。
pub const ECHRNG: i32 = 44;
/// 2 级未同步 EL2NSYNC。
pub const EL2NSYNC: i32 = 45;
/// 3 级停止 EL3HLT。
pub const EL3HLT: i32 = 46;
/// 3 级复位 EL3RST。
pub const EL3RST: i32 = 47;
/// 链接号超范围 ELNRNG。
pub const ELNRNG: i32 = 48;
/// 协议驱动未附着 EUNATCH。
pub const EUNATCH: i32 = 49;
/// 无 CSI 结构 ENOCSI。
pub const ENOCSI: i32 = 50;
/// 2 级停止 EL2HLT。
pub const EL2HLT: i32 = 51;
/// 非法交换 EBADE。
pub const EBADE: i32 = 52;
/// 非法请求描述符 EBADR。
pub const EBADR: i32 = 53;
/// 交换满 EXFULL。
pub const EXFULL: i32 = 54;
/// 无阳极 ENOANO。
pub const ENOANO: i32 = 55;
/// 非法请求码 EBADRQC。
pub const EBADRQC: i32 = 56;
/// 非法槽位 EBADSLT。
pub const EBADSLT: i32 = 57;
/// 字体文件格式错误 EBFONT。
pub const EBFONT: i32 = 59;
/// 非流设备 ENOSTR。
pub const ENOSTR: i32 = 60;
/// 无可用数据 ENODATA。
pub const ENODATA: i32 = 61;
/// 定时器到期 ETIME。
pub const ETIME: i32 = 62;
/// 流资源不足 ENOSR。
pub const ENOSR: i32 = 63;
/// 机器不在网上 ENONET。
pub const ENONET: i32 = 64;
/// 软件包未安装 ENOPKG。
pub const ENOPKG: i32 = 65;
/// 对象是远程的 EREMOTE。
pub const EREMOTE: i32 = 66;
/// 链路已断开 ENOLINK。
pub const ENOLINK: i32 = 67;
/// 广播错误 EADV。
pub const EADV: i32 = 68;
/// 挂载错误 ESRMNT。
pub const ESRMNT: i32 = 69;
/// 通信错误 ECOMM。
pub const ECOMM: i32 = 70;
/// 协议错误 EPROTO。
pub const EPROTO: i32 = 71;
/// 尝试多跳 EMULTIHOP。
pub const EMULTIHOP: i32 = 72;
/// RFS 特定错误 EDOTDOT。
pub const EDOTDOT: i32 = 73;
/// 非法消息 EBADMSG。
pub const EBADMSG: i32 = 74;
/// 值对数据类型过大 EOVERFLOW。
pub const EOVERFLOW: i32 = 75;
/// 名字在网络上不唯一 ENOTUNIQ。
pub const ENOTUNIQ: i32 = 76;
/// 文件描述符状态错误 EBADFD。
pub const EBADFD: i32 = 77;
/// 远程地址已改变 EREMCHG。
pub const EREMCHG: i32 = 78;
/// 无法访问共享库 ELIBACC。
pub const ELIBACC: i32 = 79;
/// 共享库损坏 ELIBBAD。
pub const ELIBBAD: i32 = 80;
/// 共享库 .lib 段损坏 ELIBSCN。
pub const ELIBSCN: i32 = 81;
/// 共享库过多 ELIBMAX。
pub const ELIBMAX: i32 = 82;
/// 不能直接执行共享库 ELIBEXEC。
pub const ELIBEXEC: i32 = 83;
/// 系统调用应重启 ERESTART。
pub const ERESTART: i32 = 85;
/// 流管道错误 ESTRPIPE。
pub const ESTRPIPE: i32 = 86;
/// 用户过多 EUSERS。
pub const EUSERS: i32 = 87;
/// 非套接字 ENOTSOCK。
pub const ENOTSOCK: i32 = 88;
/// 需要目标地址 EDESTADDRREQ。
pub const EDESTADDRREQ: i32 = 89;
/// 消息过长 EMSGSIZE。
pub const EMSGSIZE: i32 = 90;
/// 套接字协议类型错误 EPROTOTYPE。
pub const EPROTOTYPE: i32 = 91;
/// 协议不可用 ENOPROTOOPT。
pub const ENOPROTOOPT: i32 = 92;
/// 不支持的协议 EPROTONOSUPPORT。
pub const EPROTONOSUPPORT: i32 = 93;
/// 不支持的套接字类型 ESOCKTNOSUPPORT。
pub const ESOCKTNOSUPPORT: i32 = 94;
/// 不支持的协议族 EPFNOSUPPORT。
pub const EPFNOSUPPORT: i32 = 96;
/// 不支持的地址族 EAFNOSUPPORT。
pub const EAFNOSUPPORT: i32 = 97;
/// 地址已在使用 EADDRINUSE。
pub const EADDRINUSE: i32 = 98;
/// 无法分配请求的地址 EADDRNOTAVAIL。
pub const EADDRNOTAVAIL: i32 = 99;
/// 网络已关闭 ENETDOWN。
pub const ENETDOWN: i32 = 100;
/// 网络不可达 ENETUNREACH。
pub const ENETUNREACH: i32 = 101;
/// 网络连接被重置 ENETRESET。
pub const ENETRESET: i32 = 102;
/// 连接被中止 ECONNABORTED。
pub const ECONNABORTED: i32 = 103;
/// 连接被对端重置 ECONNRESET。
pub const ECONNRESET: i32 = 104;
/// 无可用缓冲空间 ENOBUFS。
pub const ENOBUFS: i32 = 105;
/// 套接字已连接 EISCONN。
pub const EISCONN: i32 = 106;
/// 套接字未连接 ENOTCONN。
pub const ENOTCONN: i32 = 107;
/// 发送后无法再发送 ESHUTDOWN。
pub const ESHUTDOWN: i32 = 108;
/// 引用过多 ETOOMANYREFS。
pub const ETOOMANYREFS: i32 = 109;
/// 连接超时 ETIMEDOUT。
pub const ETIMEDOUT: i32 = 110;
/// 连接被拒绝 ECONNREFUSED。
pub const ECONNREFUSED: i32 = 111;
/// 主机已关闭 EHOSTDOWN。
pub const EHOSTDOWN: i32 = 112;
/// 主机不可达 EHOSTUNREACH。
pub const EHOSTUNREACH: i32 = 113;
/// 操作已在进行 EALREADY。
pub const EALREADY: i32 = 114;
/// 操作正在进行 EINPROGRESS。
pub const EINPROGRESS: i32 = 115;
/// 陈旧的 NFS 文件句柄 ESTALE。
pub const ESTALE: i32 = 116;
/// 不是名字文件 ENOTNAM。
pub const ENOTNAM: i32 = 118;
/// 无可用 XENIX 信号量 ENAVAIL。
pub const ENAVAIL: i32 = 119;
/// 是名字文件 EISNAM。
pub const EISNAM: i32 = 120;
/// 远程 I/O 错误 EREMOTEIO。
pub const EREMOTEIO: i32 = 121;
/// 超出磁盘配额 EDQUOT。
pub const EDQUOT: i32 = 122;
/// 无介质 ENOMEDIUM。
pub const ENOMEDIUM: i32 = 123;
/// 介质类型错误 EMEDIUMTYPE。
pub const EMEDIUMTYPE: i32 = 124;
/// 操作已取消 ECANCELED。
pub const ECANCELED: i32 = 125;
/// 所需密钥不可用 ENOKEY。
pub const ENOKEY: i32 = 126;
/// 密钥已过期 EKEYEXPIRED。
pub const EKEYEXPIRED: i32 = 127;
/// 密钥已被吊销 EKEYREVOKED。
pub const EKEYREVOKED: i32 = 128;
/// 密钥被服务拒绝 EKEYREJECTED。
pub const EKEYREJECTED: i32 = 129;
/// 属主已死 EOWNERDEAD。
pub const EOWNERDEAD: i32 = 130;
/// 状态不可恢复 ENOTRECOVERABLE。
pub const ENOTRECOVERABLE: i32 = 131;
/// 因射频关闭无法操作 ERFKILL。
pub const ERFKILL: i32 = 132;
/// 内存页已损坏 EHWPOISON。
pub const EHWPOISON: i32 = 133;
/// 别名：EAGAIN（POSIX 允许二者同值） EWOULDBLOCK。
pub const EWOULDBLOCK: i32 = 11;
/// 别名：EDEADLK（同值） EDEADLOCK。
pub const EDEADLOCK: i32 = 35;
/// 别名：ENOTSUP（POSIX 允许二者同值） EOPNOTSUPP。
pub const EOPNOTSUPP: i32 = 95;

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
