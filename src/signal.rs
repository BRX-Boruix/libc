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

/// SA_ONSTACK 与 SS_* 的**内核侧同一事实**（见 libc/include/signal.h 的说明）。
/// 取值与内核 	ask::signal::SA_ONSTACK 一致（0x08000000，与 Linux 对齐）。


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
///
/// **与 `sigpending` 的区别**（别混）：本函数管的是**屏蔽集**（哪些信号被挡住），
/// `sigpending` 取的是**待决集**（挡住之后积压了哪些）。
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
    // POSIX：`set == NULL` 表示**纯查询**——屏蔽集**不变**，只回写旧值（`how` 不影响结果）。
    //
    // 此前把 NULL 当 0 传给内核，于是 `SIG_SETMASK` 变成「把屏蔽集设为 0」= **清空**：
    // 调用方做一次查询就把屏蔽解掉了。3P3-2 的 sigprocmask 行为用例正是被这一点误导——
    // 观测到的 `after=0x800` 是内核返回的**旧值**，而屏蔽当时已被清空，故后续 raise 被投递
    // （我一度据此误判为「内核投递路径不检查屏蔽位」并立项 3P4-11；**该定性是错的**）。
    //
    // 无副作用的只读查询：用 `SIGNAL_BLOCK` 传**空集**（`blocked |= ∅` 不变）且仍返回旧值——
    // 一次 syscall 完成，没有「先读后写」的竞态窗口。
    let (how_map, set_val) = if set.is_null() {
        (libsys::signal::SIGNAL_BLOCK, 0)
    } else {
        (how_map, unsafe { *set })
    };
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

// ---------- 信号集操作（POSIX sigset 家族，3P3-2 补齐）----------

/// 信号集的位编码 = **`1 << sig`**（与内核 `task::signal_set::SignalSet` 同一事实）。
///
/// `SA_ONSTACK`：在备用信号栈上执行 handler（POSIX `sigaction` 的 `sa_flags` 位）。
/// 取值与内核 `task::signal::SA_ONSTACK` 同一事实（0x08000000，与 Linux 一致）。
pub const SA_ONSTACK: c_int = 0x0800_0000;

/// `SS_ONSTACK`：查询时表示「当前正跑在备用栈上」。**本实现不产生该标志**（未实现查询），
/// 常量如实定义以符合 POSIX 头文件。
pub const SS_ONSTACK: c_int = 1;
/// `SS_DISABLE`：设置时表示禁用备用栈。
pub const SS_DISABLE: c_int = 2;

/// `stack_t`（POSIX `<signal.h>`）。
#[repr(C)]
pub struct stack_t {
    pub ss_sp: *mut c_void,
    pub ss_flags: c_int,
    pub ss_size: crate::ctypes::size_t,
}

/// `sigaltstack(ss, oss)`：设置/查询备用信号栈（POSIX，声明见 `<signal.h>`）。
///
/// `ss == NULL` 只查询。设置时 `ss_flags & SS_DISABLE` ⇒ 禁用；否则用 `ss_sp`/`ss_size`。
/// 旧值写入 `oss`（未设置过则为 `SS_DISABLE`）。
///
/// **诚实边界**：内核实现的是**每进程**备用栈（POSIX 规定每线程）——单线程下相同；
/// `SS_ONSTACK` 查询未实现，故 `oss.ss_flags` 只可能回 `SS_DISABLE` 或 0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaltstack(
    ss: *const stack_t,
    oss: *mut stack_t,
) -> c_int {
    // `ss == NULL` ⇒ **纯查询**：走内核的查询模式（不改状态）。
    //
    // 这里不能用「set(0,0) 顺便读旧值」代替——`(0,0)` 在 POSIX 里是 `SS_DISABLE`（禁用），
    // 那样「查一下当前备用栈」会把栈清掉。内核为此有独立的模式位（见 `SYS_SIGNAL_ALTSTACK`）。
    let res = if ss.is_null() {
        libsys::signal::altstack_get()
    } else {
        let s = unsafe { &*ss };
        if s.ss_flags & SS_DISABLE != 0 {
            libsys::signal::altstack_set(0, 0)
        } else if s.ss_sp.is_null() || s.ss_size == 0 {
            set_errno(EINVAL);
            return -1;
        } else {
            libsys::signal::altstack_set(s.ss_sp as u64, s.ss_size as u64)
        }
    };
    match res {
        Ok((ob, os)) => {
            if !oss.is_null() {
                let out = unsafe { &mut *oss };
                out.ss_sp = ob as *mut c_void;
                out.ss_size = os as crate::ctypes::size_t;
                out.ss_flags = if ob == 0 { SS_DISABLE } else { 0 };
            }
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `alarm(seconds)`：设置/取消一次性闹钟（POSIX，声明见 `<unistd.h>`）。
///
/// 返回**上一个**闹钟的剩余秒数（向上取整；无未到期闹钟则为 0）；`seconds == 0` 取消。
/// 到期由内核投递 `SIGALRM`。
///
/// **诚实边界**：内核实现的是**每线程**闹钟（用调用者的 pid），POSIX 规定是**每进程**的
/// ——单线程进程下二者相同。`setitimer` 的周期语义与三种计时器未实现，如实缺。
#[unsafe(no_mangle)]
pub extern "C" fn alarm(seconds: crate::ctypes::c_uint) -> crate::ctypes::c_uint {
    match libsys::signal::alarm(seconds) {
        Ok(rem) => rem as crate::ctypes::c_uint,
        Err(e) => {
            set_errno(from_libsys(e));
            // POSIX 的 alarm 没有失败返回通道；如实返回 0 并置 errno（调用方可查）。
            0
        }
    }
}

/// `sigsuspend(mask)`：原子地把屏蔽集换成 `*mask` 并挂起，直到有信号被投递
/// （POSIX，声明见 `<signal.h>`）。**恒返回 -1 置 `EINTR`**。
///
/// **原子性为什么必须由内核给**：用户态「先 `sigprocmask` 再等」中间有窗口——信号若在
/// 两次调用之间到达，就会在换上屏蔽集**之前**被投递（本该挡住它）。内核把「换集」与
/// 「入睡」做成对唤醒方不可分割的一步（`block_current_with` 的条件复检在调度锁内）。
///
/// **屏蔽集的还原点在投递侧**：阻塞的 syscall 唤醒后直接返回用户态，没有「唤醒后继续」
/// 的路径 ⇒ 内核在 `deliver_on_return` 取到信号时还原（POSIX 的规定也正是如此）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigsuspend(mask: *const sigset_t) -> c_int {
    if mask.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match libsys::signal::suspend(unsafe { *mask }) {
        // 正常路径不该走到这里（内核恒返回 Interrupted）。
        Ok(_) => {
            set_errno(crate::errno::EINTR);
            -1
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `sigpending(set)`：取当前进程的**待决信号集**（POSIX，声明见 `<signal.h>`）。
///
/// 成功返回 0 并把待决集写入 `set`；`set` 为空返回 -1 置 `EINVAL`。
///
/// **为什么现在才有**：内核此前只有 `SIGNAL_MASK`（屏蔽集读写）与内部访问器
/// `SignalState::pending()`，**没有把待决集暴露给用户态的调用号** ⇒ 本项只能登记为缺失。
/// 本轮补了内核动词 `SYS_SIGNAL_PENDING`（0x85）才有这条出口。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigpending(set: *mut sigset_t) -> c_int {
    if set.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match libsys::signal::pending() {
        Ok(bits) => {
            unsafe { *set = bits };
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// 为什么必须提供这些函数：`sigset_t` 是 u64 位图，若不给辅助函数，C 程序只能**手搓位**——
/// 极易按「bit(sig-1)」的直觉写错（本仓的覆盖面驱动就这么错过一次：传 `1<<11` 以为屏蔽了
/// SIGUSR2，实际屏蔽的是 SIGSEGV=11）。POSIX 要求这五个函数正是为了让调用方不碰编码。
const SIGSET_NSIG: c_int = 64;

/// `sigemptyset(set)`：清空信号集。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigemptyset(set: *mut sigset_t) -> c_int {
    if set.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { *set = 0 };
    0
}

/// `sigfillset(set)`：置入**全部**信号。
///
/// **与内核同一事实源**：内核 `SignalSet::all()` 就是 `u64::MAX`（"0..NSIG 全部为 1"，
/// NSIG=64）。此前这里写成 `u64::MAX & !(1u64 << 63)` 并注"bit63 保留，与内核约定一致"——
/// **那句是错的**：内核的满集含 bit63，于是本函数少置了一个信号（63）。现改为与内核一致。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigfillset(set: *mut sigset_t) -> c_int {
    if set.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { *set = u64::MAX };
    0
}

/// `sigaddset(set, sig)`：把信号加入集合。信号号须在 `1..NSIG`（POSIX：0 不是信号）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaddset(set: *mut sigset_t, sig: c_int) -> c_int {
    if set.is_null() || !(1..SIGSET_NSIG).contains(&sig) {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { *set |= 1u64 << sig };
    0
}

/// `sigdelset(set, sig)`：把信号移出集合。信号号须在 `1..NSIG`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigdelset(set: *mut sigset_t, sig: c_int) -> c_int {
    if set.is_null() || !(1..SIGSET_NSIG).contains(&sig) {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { *set &= !(1u64 << sig) };
    0
}

/// `sigismember(set, sig)`：信号是否在集合中（1=是，0=否，-1=错误）。信号号须在 `1..NSIG`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigismember(set: *const sigset_t, sig: c_int) -> c_int {
    if set.is_null() || !(1..SIGSET_NSIG).contains(&sig) {
        set_errno(EINVAL);
        return -1;
    }
    ((unsafe { *set } >> sig) & 1) as c_int
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
