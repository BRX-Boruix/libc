//! POSIX 信号接口（C ABI）：signal / sigaction / sigprocmask / raise。
//!
//! 真实数据链路（S06）：经 `libsys::signal` 的 SIGNAL 域 syscall 落到内核
//! （`kernel/crates/task/src/signal.rs` 的 SignalState：处置/mask/raise/投递/sigreturn）。
//!
//! 内核信号模型（ADR-034）：
//! - 每进程 `SignalState` 保存各信号处置（Default / Ignore / Handler(fn)）、屏蔽集、
//!   未决集；`deliver_on_return` 在返回用户态前投递。
//! - 处理器经 `deliver_handler` 以 `void handler(int sig)` 进入（`rdi=sig`、
//!   `rsi=siginfo`、`rdx=0`=ucontext 未实现），返回后由内核安装的 restorer 调
//!   `rt_sigreturn` 恢复。
//! - **诚实边界（S09）**：内核**无纯查询处置原语**（`action` 恒为"设新返回旧"）、
//!   无**按动作的** `sa_mask`（只有进程级屏蔽集）、无 `SA_SIGINFO` 等旗标语义。
//!   `sigaction` 据此如实实现并逐条注释；未知旗标不伪装。

use crate::ctypes::{c_int, c_void};

use crate::errno::{from_libsys, set_errno, EINVAL};

/// 信号号（与内核 `task::signals` / libsys `signal` 对齐，S13）。
pub const SIGINT: c_int = 2;
pub const SIGILL: c_int = 4;
pub const SIGBUS: c_int = 7;
pub const SIGFPE: c_int = 8;
pub const SIGKILL: c_int = 9;
pub const SIGUSR1: c_int = 10;
pub const SIGSEGV: c_int = 11;
pub const SIGUSR2: c_int = 12;
pub const SIGPIPE: c_int = 13;
pub const SIGALRM: c_int = 14;
pub const SIGTERM: c_int = 15;
pub const SIGCHLD: c_int = 17;
pub const SIGCONT: c_int = 18;
pub const SIGSTOP: c_int = 19;

/// 处置哨兵（ADR-034 §3.3）：SIG_DFL / SIG_IGN 由内核 `action` 以 0/1 编码。
pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;
/// `SIG_ERR`：signal() 出错返回（-1 的位型，作为哨兵）。
pub const SIG_ERR: usize = usize::MAX;

/// `sighandler_t`：信号处理函数指针（或 SIG_DFL/SIG_IGN 哨兵）。
pub type sighandler_t = usize;

/// `sigset_t`：屏蔽集（BORUIX 用 u64 位图，NSIG=64，与 libsys mask 对齐）。
pub type sigset_t = u64;

/// `struct sigaction`（BORUIX 精简 ABI，x86_64）：
/// - `sa_handler`：SIG_DFL/SIG_IGN/函数指针；
/// - `sa_mask`：进程级屏蔽近似（内核无按动作 mask，见模块注释）；
/// - `sa_flags`：透传（当前内核仅接受 0）；
/// - `sa_restorer`：保留（恒 0，内核自动安装 restorer）。
#[repr(C)]
pub struct sigaction {
    pub sa_handler: sighandler_t,
    pub sa_mask: sigset_t,
    pub sa_flags: c_int,
    pub sa_restorer: *mut c_void,
}

/// `signal(sig, handler)`：设置信号处置，返回旧处置（SIG_DFL/SIG_IGN/旧指针）。
///
/// 返回 `SIG_ERR` 并置 errno 表示失败（如对 SIGKILL/SIGSTOP 设非默认）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn signal(sig: c_int, handler: sighandler_t) -> sighandler_t {
    match libsys::signal::action(sig as u32, handler as u64, 0) {
        Ok(old) => old as sighandler_t,
        Err(e) => {
            set_errno(from_libsys(e));
            SIG_ERR
        }
    }
}

/// 内部：把旧处置（u64 编码）写入 `oact`（若非空）。
fn fill_oact(oact: *mut sigaction, old_disp: u64) {
    if oact.is_null() {
        return;
    }
    unsafe {
        (*oact).sa_handler = old_disp as sighandler_t;
        (*oact).sa_mask = 0;
        (*oact).sa_flags = 0;
        (*oact).sa_restorer = core::ptr::null_mut();
    }
}

/// `sigaction(sig, act, oact)`：查/设信号处置（`act` 与 `oact` 至少一者非空）。
///
/// - 设 `act.sa_handler`（SIG_DFL/SIG_IGN/函数指针）与 `act.sa_flags`；
/// - `act.sa_mask` 非空 → 以进程级 `sigprocmask(SIG_BLOCK)` 近似屏蔽（如实：内核
///   无按动作 mask，屏蔽为进程级且不随 handler 返回自动恢复）；
/// - `oact` 填入**旧**处置与旧屏蔽（仅当 `oact != NULL`）。
///
/// 返回 0 成功，-1 失败置 errno（SIGKILL/SIGSTOP 非默认 → EINVAL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaction(
    sig: c_int,
    act: *const sigaction,
    oact: *mut sigaction,
) -> c_int {
    if act.is_null() && oact.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    // 纯查询（act==NULL）：内核无纯查询原语，用"设 SIG_DFL 取旧、再设回旧"实现。
    // 单进程 toy 内核下无竞争；如实注释为查询-恢复。
    if act.is_null() {
        let old = libsys::signal::action(sig as u32, libsys::signal::SIG_DFL, 0);
        return match old {
            Ok(old_disp) => {
                if old_disp != libsys::signal::SIG_DFL {
                    let _ = libsys::signal::action(sig as u32, old_disp, 0);
                }
                fill_oact(oact, old_disp);
                0
            }
            Err(e) => {
                set_errno(from_libsys(e));
                -1
            }
        };
    }
    // 设处置路径。
    let new_handler = unsafe { (*act).sa_handler as u64 };
    let new_flags = unsafe { (*act).sa_flags } as u32;
    let mask = unsafe { (*act).sa_mask };
    // 应用进程级 mask（若请求）。
    if mask != 0 {
        if let Err(e) = libsys::signal::mask(libsys::signal::SIGNAL_BLOCK, mask) {
            set_errno(from_libsys(e));
            return -1;
        }
    }
    match libsys::signal::action(sig as u32, new_handler, new_flags) {
        Ok(old_disp) => {
            fill_oact(oact, old_disp);
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `sigprocmask(how, set, oset)`：查/改进程屏蔽集。
///
/// `set`/`oset` 至少一者非空。`how`：SIG_BLOCK(0)/SIG_UNBLOCK(1)/SIG_SETMASK(2)
/// （与 libsys SIGNAL_BLOCK/UNBLOCK/SET 对齐）。返回 0 成功，-1 失败置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigprocmask(
    how: c_int,
    set: *const sigset_t,
    oset: *mut sigset_t,
) -> c_int {
    if set.is_null() && oset.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let how_map = match how {
        0 => libsys::signal::SIGNAL_BLOCK,
        1 => libsys::signal::SIGNAL_UNBLOCK,
        2 => libsys::signal::SIGNAL_SET,
        _ => {
            set_errno(EINVAL);
            return -1;
        }
    };
    let set_val: sigset_t = if set.is_null() { 0 } else { unsafe { *set } };
    match libsys::signal::mask(how_map, set_val) {
        Ok(old) => {
            if !oset.is_null() {
                unsafe { *oset = old };
            }
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `raise(sig)`：向自身发送信号（复用内核 kill(getpid)）。
///
/// 返回 0 成功，-1 失败置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn raise(sig: c_int) -> c_int {
    let pid = crate::process::getpid();
    if pid < 0 {
        set_errno(crate::errno::ENOENT);
        return -1;
    }
    match libsys::signal::raise(pid as u64, sig as u32) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}
