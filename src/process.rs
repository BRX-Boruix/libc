//! 进程函数（C ABI）：exit / getpid / gettid / kill / waitpid / yield。
//!
//! ## getpid/gettid 的实现（threads.md T2-6，S09 如实）
//!
//! 内核提供 `SYS_TASK_GETPID`（0x39，返回所在线程组组长 pid / POSIX 进程 id）与
//! `SYS_TASK_GETTID`（0x38，返回调用线程自身 pid）。本模块直接走内核 syscall，
//! 不再读 ProcFS `/processes/list` 扫 Running（多线程/SMP 下会挑错成员）。
//! 单线程进程 tgid==pid；多线程时组员 getpid==组长 pid、gettid==自身 pid。

use crate::ctypes::c_int;
use crate::errno::{set_errno, from_libsys};

/// `exit(code)`：终止当前进程。永不返回。
///
/// POSIX：`exit` 先按 LIFO 调用 `atexit` 登记的处理函数，再真正退出；`_exit` 不调用。
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: c_int) -> ! {
    crate::stdlib::run_atexit_handlers();
    libsys::exit(code)
}

/// `_exit(code)`：立即终止，**不**运行 `atexit` 处理函数、不做任何清理（POSIX 语义）。
#[unsafe(no_mangle)]
pub extern "C" fn _exit(code: c_int) -> ! {
    libsys::exit(code)
}

/// `gettid()`：返回调用线程自身的线程 id（= 内核 pid；组长 pid==tgid、组员 pid==线程 id）。
/// 线程可据此把本线程 id 写进其 `Tcb.tid`。失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn gettid() -> c_int {
    match libsys::gettid() {
        Ok(t) => t as c_int,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// `getpid()`：返回当前进程（线程组组长）PID = 所在组 tgid（POSIX 进程 id）。
/// 走内核 `SYS_TASK_GETPID`（0x39）。失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> c_int {
    match libsys::getpid() {
        Ok(t) => t as c_int,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// 查询本进程真实身份（uid/gid/caps）的**单一入口**。
///
/// 四项 get*id 的 POSIX 语义完全一致（查不到才置 errno），故只在这里查一次、只写一份错误路径。
/// 失败时返回 None（**不**返回 0——0 是 root 的合法 uid，兜底就等于伪造身份，S09 不允许）。
fn query_identity() -> Option<libsys::IdentityInfo> {
    match libsys::identity_query() {
        Ok(i) => Some(i),
        Err(e) => {
            set_errno(from_libsys(e));
            None
        }
    }
}

/// getuid()：本进程的**真实** uid。
///
/// **来路（3P6-2 第二波，「整项缺失」类）**：内核早有 identity_query
/// （IdentityInfo 12 字节，uid@0 / gid@4 / caps@8，编译期断言钉死），libsys 也已公开
/// re-export（libsys::identity_query）——缺的只是 libc 这一层。这类「能力已存在、C 面没暴露」
/// 的缺口是**查不出来**的（名字审计只看已导出项），故新增 libc/tools/audit_posix_surface.py
/// 做反向对账（清单 vs 实现）。
///
/// **诚实边界（S09）**：本系统内核只维护**一份** uid/gid，**没有** real/effective 之分，
/// 故 geteuid() 与 getuid() 返回同一值（gid 同理）——这是事实陈述，不是偷懒。
/// POSIX 规定本函数不会失败；内核仅在「无当前进程」的内核上下文返回 PermissionDenied，
/// 用户进程走不到，故此处的 (uid_t)-1 是**防御性**分支而非正常返回。
#[unsafe(no_mangle)]
pub extern "C" fn getuid() -> crate::ctypes::uid_t {
    match query_identity() {
        Some(i) => i.uid,
        None => (-1i32) as crate::ctypes::uid_t,
    }
}

/// geteuid()：有效 uid。本系统无 real/effective 之分，故与 getuid() 同值（见其说明）。
#[unsafe(no_mangle)]
pub extern "C" fn geteuid() -> crate::ctypes::uid_t {
    getuid()
}

/// getgid()：本进程的**真实** gid。
#[unsafe(no_mangle)]
pub extern "C" fn getgid() -> crate::ctypes::gid_t {
    match query_identity() {
        Some(i) => i.gid,
        None => (-1i32) as crate::ctypes::gid_t,
    }
}

/// getegid()：有效 gid。本系统无 real/effective 之分，故与 getgid() 同值。
#[unsafe(no_mangle)]
pub extern "C" fn getegid() -> crate::ctypes::gid_t {
    getgid()
}

/// getppid()：父进程 pid。
///
/// **来路（3P6-2 第二波「整项缺失」类）**：父 pid 由内核经 procfs 暴露
/// （`vfs/src/procfs.rs` 的 ppid 字段），libsys 已有 `ps_list()` 把它解析成 `PsEntry`——
/// 缺的只是 libc 这一层。
///
/// **诚实边界（S09）**：数据来自 procfs 的**一次快照读**；返回的是快照时刻的真实父子关系，
/// 不是缓存、不是猜测。POSIX 规定本函数不会失败，故查不到时返回 0（防御性分支；正常进程
/// 一定在进程表里）。
#[unsafe(no_mangle)]
pub extern "C" fn getppid() -> c_int {
    let me = match libsys::getpid() {
        Ok(p) => p,
        Err(e) => {
            set_errno(from_libsys(e));
            return 0;
        }
    };
    match libsys::ps_list() {
        Ok(list) => {
            for e in list.iter() {
                if e.pid as u64 == me {
                    return e.ppid as c_int;
                }
            }
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            0
        }
    }
}

/// `kill(pid, sig)`：向进程发信号。返回 0 或 -1（置 errno）。
#[unsafe(no_mangle)]
pub extern "C" fn kill(pid: c_int, sig: c_int) -> c_int {
    match libsys::kill(pid as u64, sig as u64) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `waitpid(pid, status, options)`：等待子进程退出。
///
/// 返回被收尸子进程的 **pid**（POSIX 语义；失败返回 -1 并置 errno）。退出码
/// 写入 `*status` 高 8 位（WEXITSTATUS 语义）。
///
/// - `pid`：内核 libsys 仅支持等任意子进程（`waitpid_any`）。`pid>0`
///   精确匹配内核无此原语，`pid==0`（同进程组）本系统无进程组概念——两者
///   均如实映射为"等任意子"（与内核能力一致，S09 不伪装精确匹配）。
/// - `options` 非 0（WNOHANG/WUNTRACED 等）暂不支持，如实返回 -1 置 ENOTSUP。
#[unsafe(no_mangle)]
pub extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    let _ = pid;
    // WNOHANG：非阻塞轮询（POSIX）。本系统内核支持「有界等待」——`target>0 && timeout>0` 到期时
    // 子进程仍在运行则如实返回 `WouldBlock`（见 kernel syscall.rs 的 sys_task_wait 文档），
    // 故这里用 1ns 的超时实现「立即返回」。
    //
    // **诚实边界（S09）**：受内核定时器粒度限制，1ns 超时实际可能短暂等待（远小于一次调度片），
    // 不是严格的「零等待」；但语义正确（没有子进程已退出就返回 0）。
    //
    // 来路：目标第 7/8 轮——它是 libc 宽度的合法缺口（此前 options != 0 一律 ENOTSUP），
    // 同时充当「子进程卡在 _exit」与「父 wait 丢唤醒」的判别器。
    if options & crate::unistd::WNOHANG != 0 {
        return match libsys::waitpid_any_timeout(1) {
            Ok(wr) => {
                if wr.pid == 0 {
                    0
                } else {
                    if !status.is_null() {
                        unsafe { *status = ((wr.code & 0xFF) as c_int) << 8; }
                    }
                    wr.pid as c_int
                }
            }
            Err(libsys::Error::WouldBlock) => 0,
            Err(e) => {
                set_errno(from_libsys(e));
                -1
            }
        };
    }
    if options != 0 {
        set_errno(crate::errno::ENOTSUP);
        return -1;
    }
    match libsys::waitpid_any() {
        Ok(wr) => {
            if !status.is_null() {
                unsafe {
                    *status = ((wr.code & 0xFF) as c_int) << 8;
                }
            }
            wr.pid as c_int
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `fork()`：创建一个子进程（COW 语义）。
///
/// # 为什么这不是 POSIX `fork()` 的逐字复刻（诚实声明，S39）
///
/// 本实现的**语义**是 POSIX fork：调用一次、返回两次；父收子 pid（> 0）、子收 0；
/// 失败父收 -1 置 errno。子进程从**本函数内部**的返回点继续执行，拥有父在调用
/// 时刻的全部内存映像（写时复制）与 fd/cwd/identity 拷贝。
///
/// 但有三条**明确的差异**，调用方必须知晓：
///
/// 1. **多线程父进程被内核拒绝**（`ENOTSUP`）。POSIX fork 在子进程内只保留调用
///    线程，其余线程消失；那些线程可能正持有互斥锁/维护着全局不变式，子进程里
///    这些锁将永久无人释放。ADR-038 决策 4 选择**如实拒绝**而非静默产出语义残缺
///    的子进程——宁可让调用方收到错误，也不给它一个会随机死锁的进程。
/// 2. **无 atexit / 无 stdio 缓冲复制**：与 `exit()` 一致，本实现无 atexit 处理器；
///    libc 的 stdio 缓冲若需在 fork 后清空，由调用方自行 `fflush` 处理。
/// 3. **信号处置**按 ADR-034/ADR-038 决策 5：子进程继承父的 restorer 地址与信号
///    状态快照，但**不**继承父的未决信号掩码中的"正在处理"重入守卫。
///
/// # 返回
///
/// - 父进程：新子进程的 pid（`> 0`）；
/// - 子进程：`0`；
/// - 失败：`-1` 并置 `errno`（父进程内，子进程不存在）。
///
/// # 实现注意（为何函数体里没有锁、没有 `static mut`）
///
/// 本函数**会返回两次**。任何在返回点之前持有、且预期在返回点之后仍一致的资源
/// （全局锁、`static mut` 缓存、可变借用）都会在子进程里处于"被父进程持有"的
/// 假象状态。故本实现刻意保持**无状态**：只调一个 syscall 并立即返回，不触碰任何
/// 全局可变状态。`errno` 的设置也只在失败路径（那时代码只在父进程内执行，子进程
/// 从未被创建），故不存在"子进程误继承父的 errno"的问题。
#[unsafe(no_mangle)]
pub extern "C" fn fork() -> c_int {
    // 三个保留参数显式钉为 0（继承父当前 RIP/RSP）——语义写在调用点，
    // 不依赖调用方记得传 0（S13：不留魔法值）。
    match libsys::derive_inherit() {
        Ok(pid) => pid as c_int,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `yield()`：主动让出 CPU。
#[unsafe(no_mangle)]
pub extern "C" fn yield_sys() -> c_int {
    match libsys::yield_now() {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}
