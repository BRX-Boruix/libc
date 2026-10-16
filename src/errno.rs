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
pub use crate::abi_generated::EINVAL;
/// 数值越界 ERANGE。
pub use crate::abi_generated::ERANGE;
/// 功能未实现 ENOSYS（POSIX；用于「本系统没有该能力」的**如实**拒绝，
/// 例如 `posix_spawnattr_setflags` 的进程组/信号屏蔽/调度优先级旗标）。
pub use crate::abi_generated::ENOSYS;
/// 目标不存在 ENOENT。
pub use crate::abi_generated::ENOENT;
/// 目标已存在 EEXIST。
pub use crate::abi_generated::EEXIST;
/// 不支持的操作 ENOTSUP。
pub use crate::abi_generated::ENOTSUP;
/// 非阻塞操作无法立即完成 EAGAIN。
pub use crate::abi_generated::EAGAIN;
/// 阻塞中的系统调用被信号打断 EINTR（ADR-051）。
pub use crate::abi_generated::EINTR;
/// 空间不足 ENOSPC。
pub use crate::abi_generated::ENOSPC;
/// 设备 I/O 错误 EIO。
pub use crate::abi_generated::EIO;
/// 内存耗尽 ENOMEM。
pub use crate::abi_generated::ENOMEM;
/// 无效文件描述符 EBADF。
pub use crate::abi_generated::EBADF;
/// 文件描述符表满 EMFILE。
pub use crate::abi_generated::EMFILE;
/// 参数列表太长 / 超出范围 E2BIG。
pub use crate::abi_generated::E2BIG;
/// 打开的文件过多 ENFILE。
pub use crate::abi_generated::ENFILE;
/// 目录非空 ENOTEMPTY。
pub use crate::abi_generated::ENOTEMPTY;
/// 不是目录 ENOTDIR。
pub use crate::abi_generated::ENOTDIR;
/// 是目录 EISDIR。
pub use crate::abi_generated::EISDIR;
/// `EILSEQ`：非法字节序列（宽字符转换失败）。
pub use crate::abi_generated::EILSEQ;
/// 可执行格式非法 ENOEXEC。
pub use crate::abi_generated::ENOEXEC;
/// 权限不足 EACCES。
pub use crate::abi_generated::EACCES;
/// 地址非法 EFAULT。
pub use crate::abi_generated::EFAULT;
/// 名称过长 ENAMETOOLONG。
pub use crate::abi_generated::ENAMETOOLONG;
/// 符号链接层数过多 ELOOP。
pub use crate::abi_generated::ELOOP;
/// 非法 seek ESPIPE。
pub use crate::abi_generated::ESPIPE;
/// 只读文件系统 EROFS。
pub use crate::abi_generated::EROFS;
/// 操作不允许 EPERM。
///
/// **边界已闭合（登记项⑤）**：内核新增了 `NotPermitted`（EPERM）——它与
/// `PermissionDenied`（EACCES）的分界是「有这个权限也不许这么做」vs「你没有这个权限」，
/// 典型是 POSIX 对 `unlink(<目录>)` 的规定码。此前内核只有 `PermissionDenied`（→EACCES），
/// 故内核错误**永远不会产生 `EPERM`**——本常量当时存在只是为了让 C 程序
/// 能比较（头文件一直有 `#define EPERM`，而 Rust 侧此前没有，两侧不对称，由
/// `libc/tools/audit_errno_sync.py` 列出）。
pub use crate::abi_generated::EPERM;
/// 资源忙 EBUSY。
pub use crate::abi_generated::EBUSY;
/// 文件系统结构损坏 EUCLEAN。
pub use crate::abi_generated::EUCLEAN;

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
pub use crate::abi_generated::ESRCH;
/// 无此设备或地址 ENXIO。
pub use crate::abi_generated::ENXIO;
/// 无子进程 ECHILD。
pub use crate::abi_generated::ECHILD;
/// 需要块设备 ENOTBLK。
pub use crate::abi_generated::ENOTBLK;
/// 跨设备链接 EXDEV。
pub use crate::abi_generated::EXDEV;
/// 无此设备 ENODEV。
pub use crate::abi_generated::ENODEV;
/// 不适当的 ioctl ENOTTY。
pub use crate::abi_generated::ENOTTY;
/// 文本文件忙 ETXTBSY。
pub use crate::abi_generated::ETXTBSY;
/// 文件过大 EFBIG。
pub use crate::abi_generated::EFBIG;
/// 链接数过多 EMLINK。
pub use crate::abi_generated::EMLINK;
/// 管道破裂 EPIPE。
pub use crate::abi_generated::EPIPE;
/// 数学参数超出定义域 EDOM。
pub use crate::abi_generated::EDOM;
/// 资源死锁 EDEADLK。
pub use crate::abi_generated::EDEADLK;
/// 无可用锁 ENOLCK。
pub use crate::abi_generated::ENOLCK;
/// 无期望类型的消息 ENOMSG。
pub use crate::abi_generated::ENOMSG;
/// 标识符已删除 EIDRM。
pub use crate::abi_generated::EIDRM;
/// 通道号超范围 ECHRNG。
pub use crate::abi_generated::ECHRNG;
/// 2 级未同步 EL2NSYNC。
pub use crate::abi_generated::EL2NSYNC;
/// 3 级停止 EL3HLT。
pub use crate::abi_generated::EL3HLT;
/// 3 级复位 EL3RST。
pub use crate::abi_generated::EL3RST;
/// 链接号超范围 ELNRNG。
pub use crate::abi_generated::ELNRNG;
/// 协议驱动未附着 EUNATCH。
pub use crate::abi_generated::EUNATCH;
/// 无 CSI 结构 ENOCSI。
pub use crate::abi_generated::ENOCSI;
/// 2 级停止 EL2HLT。
pub use crate::abi_generated::EL2HLT;
/// 非法交换 EBADE。
pub use crate::abi_generated::EBADE;
/// 非法请求描述符 EBADR。
pub use crate::abi_generated::EBADR;
/// 交换满 EXFULL。
pub use crate::abi_generated::EXFULL;
/// 无阳极 ENOANO。
pub use crate::abi_generated::ENOANO;
/// 非法请求码 EBADRQC。
pub use crate::abi_generated::EBADRQC;
/// 非法槽位 EBADSLT。
pub use crate::abi_generated::EBADSLT;
/// 字体文件格式错误 EBFONT。
pub use crate::abi_generated::EBFONT;
/// 非流设备 ENOSTR。
pub use crate::abi_generated::ENOSTR;
/// 无可用数据 ENODATA。
pub use crate::abi_generated::ENODATA;
/// 定时器到期 ETIME。
pub use crate::abi_generated::ETIME;
/// 流资源不足 ENOSR。
pub use crate::abi_generated::ENOSR;
/// 机器不在网上 ENONET。
pub use crate::abi_generated::ENONET;
/// 软件包未安装 ENOPKG。
pub use crate::abi_generated::ENOPKG;
/// 对象是远程的 EREMOTE。
pub use crate::abi_generated::EREMOTE;
/// 链路已断开 ENOLINK。
pub use crate::abi_generated::ENOLINK;
/// 广播错误 EADV。
pub use crate::abi_generated::EADV;
/// 挂载错误 ESRMNT。
pub use crate::abi_generated::ESRMNT;
/// 通信错误 ECOMM。
pub use crate::abi_generated::ECOMM;
/// 协议错误 EPROTO。
pub use crate::abi_generated::EPROTO;
/// 尝试多跳 EMULTIHOP。
pub use crate::abi_generated::EMULTIHOP;
/// RFS 特定错误 EDOTDOT。
pub use crate::abi_generated::EDOTDOT;
/// 非法消息 EBADMSG。
pub use crate::abi_generated::EBADMSG;
/// 值对数据类型过大 EOVERFLOW。
pub use crate::abi_generated::EOVERFLOW;
/// 名字在网络上不唯一 ENOTUNIQ。
pub use crate::abi_generated::ENOTUNIQ;
/// 文件描述符状态错误 EBADFD。
pub use crate::abi_generated::EBADFD;
/// 远程地址已改变 EREMCHG。
pub use crate::abi_generated::EREMCHG;
/// 无法访问共享库 ELIBACC。
pub use crate::abi_generated::ELIBACC;
/// 共享库损坏 ELIBBAD。
pub use crate::abi_generated::ELIBBAD;
/// 共享库 .lib 段损坏 ELIBSCN。
pub use crate::abi_generated::ELIBSCN;
/// 共享库过多 ELIBMAX。
pub use crate::abi_generated::ELIBMAX;
/// 不能直接执行共享库 ELIBEXEC。
pub use crate::abi_generated::ELIBEXEC;
/// 系统调用应重启 ERESTART。
pub use crate::abi_generated::ERESTART;
/// 流管道错误 ESTRPIPE。
pub use crate::abi_generated::ESTRPIPE;
/// 用户过多 EUSERS。
pub use crate::abi_generated::EUSERS;
/// 非套接字 ENOTSOCK。
pub use crate::abi_generated::ENOTSOCK;
/// 需要目标地址 EDESTADDRREQ。
pub use crate::abi_generated::EDESTADDRREQ;
/// 消息过长 EMSGSIZE。
pub use crate::abi_generated::EMSGSIZE;
/// 套接字协议类型错误 EPROTOTYPE。
pub use crate::abi_generated::EPROTOTYPE;
/// 协议不可用 ENOPROTOOPT。
pub use crate::abi_generated::ENOPROTOOPT;
/// 不支持的协议 EPROTONOSUPPORT。
pub use crate::abi_generated::EPROTONOSUPPORT;
/// 不支持的套接字类型 ESOCKTNOSUPPORT。
pub use crate::abi_generated::ESOCKTNOSUPPORT;
/// 不支持的协议族 EPFNOSUPPORT。
pub use crate::abi_generated::EPFNOSUPPORT;
/// 不支持的地址族 EAFNOSUPPORT。
pub use crate::abi_generated::EAFNOSUPPORT;
/// 地址已在使用 EADDRINUSE。
pub use crate::abi_generated::EADDRINUSE;
/// 无法分配请求的地址 EADDRNOTAVAIL。
pub use crate::abi_generated::EADDRNOTAVAIL;
/// 网络已关闭 ENETDOWN。
pub use crate::abi_generated::ENETDOWN;
/// 网络不可达 ENETUNREACH。
pub use crate::abi_generated::ENETUNREACH;
/// 网络连接被重置 ENETRESET。
pub use crate::abi_generated::ENETRESET;
/// 连接被中止 ECONNABORTED。
pub use crate::abi_generated::ECONNABORTED;
/// 连接被对端重置 ECONNRESET。
pub use crate::abi_generated::ECONNRESET;
/// 无可用缓冲空间 ENOBUFS。
pub use crate::abi_generated::ENOBUFS;
/// 套接字已连接 EISCONN。
pub use crate::abi_generated::EISCONN;
/// 套接字未连接 ENOTCONN。
pub use crate::abi_generated::ENOTCONN;
/// 发送后无法再发送 ESHUTDOWN。
pub use crate::abi_generated::ESHUTDOWN;
/// 引用过多 ETOOMANYREFS。
pub use crate::abi_generated::ETOOMANYREFS;
/// 连接超时 ETIMEDOUT。
pub use crate::abi_generated::ETIMEDOUT;
/// 连接被拒绝 ECONNREFUSED。
pub use crate::abi_generated::ECONNREFUSED;
/// 主机已关闭 EHOSTDOWN。
pub use crate::abi_generated::EHOSTDOWN;
/// 主机不可达 EHOSTUNREACH。
pub use crate::abi_generated::EHOSTUNREACH;
/// 操作已在进行 EALREADY。
pub use crate::abi_generated::EALREADY;
/// 操作正在进行 EINPROGRESS。
pub use crate::abi_generated::EINPROGRESS;
/// 陈旧的 NFS 文件句柄 ESTALE。
pub use crate::abi_generated::ESTALE;
/// 不是名字文件 ENOTNAM。
pub use crate::abi_generated::ENOTNAM;
/// 无可用 XENIX 信号量 ENAVAIL。
pub use crate::abi_generated::ENAVAIL;
/// 是名字文件 EISNAM。
pub use crate::abi_generated::EISNAM;
/// 远程 I/O 错误 EREMOTEIO。
pub use crate::abi_generated::EREMOTEIO;
/// 超出磁盘配额 EDQUOT。
pub use crate::abi_generated::EDQUOT;
/// 无介质 ENOMEDIUM。
pub use crate::abi_generated::ENOMEDIUM;
/// 介质类型错误 EMEDIUMTYPE。
pub use crate::abi_generated::EMEDIUMTYPE;
/// 操作已取消 ECANCELED。
pub use crate::abi_generated::ECANCELED;
/// 所需密钥不可用 ENOKEY。
pub use crate::abi_generated::ENOKEY;
/// 密钥已过期 EKEYEXPIRED。
pub use crate::abi_generated::EKEYEXPIRED;
/// 密钥已被吊销 EKEYREVOKED。
pub use crate::abi_generated::EKEYREVOKED;
/// 密钥被服务拒绝 EKEYREJECTED。
pub use crate::abi_generated::EKEYREJECTED;
/// 属主已死 EOWNERDEAD。
pub use crate::abi_generated::EOWNERDEAD;
/// 状态不可恢复 ENOTRECOVERABLE。
pub use crate::abi_generated::ENOTRECOVERABLE;
/// 因射频关闭无法操作 ERFKILL。
pub use crate::abi_generated::ERFKILL;
/// 内存页已损坏 EHWPOISON。
pub use crate::abi_generated::EHWPOISON;
/// 别名：EAGAIN（POSIX 允许二者同值） EWOULDBLOCK。
pub use crate::abi_generated::EWOULDBLOCK;
/// 别名：EDEADLK（同值） EDEADLOCK。
pub use crate::abi_generated::EDEADLOCK;
/// 别名：ENOTSUP（POSIX 允许二者同值） EOPNOTSUPP。
pub use crate::abi_generated::EOPNOTSUPP;

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
        // 坏 fd（不存在 / 打开方向不支持本次操作）——POSIX 的规定码。
        libsys::Error::BadFd => EBADF,
        // 没有可等待的子进程——POSIX 的规定码（登记项⑧）。
        libsys::Error::NoChild => ECHILD,
        // 操作不允许——与 EACCES（PermissionDenied）**不同**的规定码（登记项⑤）。
        libsys::Error::NotPermitted => EPERM,
        libsys::Error::Unknown(n) => n,
    }
}
