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
