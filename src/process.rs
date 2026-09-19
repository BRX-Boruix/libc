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
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: c_int) -> ! {
    libsys::exit(code)
}

/// `_exit(code)`：与 exit 等价（本实现无 atexit/清理）。
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
