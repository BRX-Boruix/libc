//! `posix_spawn` 家族（POSIX.1-2008）：**不替换当前映像**地派生一个跑指定程序的新进程。
//!
//! ## 为什么这是 GCC 宿主端口的解锁项
//!
//! GCC 的 driver 用 **exec 替换**去跑 `cc1`/`as`/`ld`，而本系统**没有替换进程映像的 syscall**
//! （只有派生）。POSIX 为此提供的正是 `posix_spawn`——**它本来就不要求替换**。
//!
//! ## 基座：`SYS_TASK_SPAWN`（= `libsys::exec_path`），**不是** `SYS_TASK_DERIVE`
//!
//! 更正一处流传的错标：`SYS_TASK_DERIVE` 是 **fork（COW 派生子进程）**（见内核 `sys_task_derive`
//! 的文档）；「派生一个跑某程序的进程」是 **`SYS_TASK_SPAWN`**，它的实现函数在内核里**就叫
//! `sys_exec`**（`kernel/crates/kernel/src/syscall.rs:5428`）。
//! 用 DERIVE 做 `posix_spawn` **走不通**：fork 之后要替换映像，而那个 syscall 不存在。
//!
//! ## 诚实边界（S09，逐条可复核）
//!
//! 1. **参数含空白无法无损表达**：本系统的入口 ABI 是「**argv[0] = 整条命令行**」（不是 argv 数组），
//!    而命令行只按空白切词（`libsys::split_words`，**不支持引号**）。故 `argv[1..]` 以空格连接；
//!    若某个参数本身含空格/制表符，**如实返回 `EINVAL`**——绝不静默把它切成两个参数
//!    （那会让调用方以为 `-DFOO=a b` 生效了）。
//! 2. **`envp` 被忽略**：本系统的环境是每次派生时由内核从**权威状态重建**（`build_boot_env`），
//!    用户态 `setenv` 与显式 `envp` 传递**尚未实现**（`3P4-2b`，`3p.md` 已登记）。
//!    故 `posix_spawn` 无法把调用方的 `envp` 交给子进程——**如实声明**，不假装接受。
//! 3. **`attrp` 只支持 `POSIX_SPAWN_RESETIDS`**（且在本系统里是**无操作**：没有 real/effective 之分）。
//!    其余旗标（`SETPGROUP`/`SETSIGMASK`/`SETSIGDEF`/`SETSCHED*`）需要进程组/信号栈/调度优先级，
//!    本系统没有 ⇒ **如实返回 `ENOSYS`**，绝不静默忽略。
//! 4. **`file_actions` 在父进程里施加后还原**（不是子进程侧施加——没有 fork+exec 两步）：
//!    为每个被触及的 fd 先用 `fcntl(F_DUPFD, 64)` 备份（备份 fd ≥ 64，避开常规 fd 号），
//!    派生后按**逆序**还原。**并发警告**：多线程父进程里这段窗口对其他线程可见——
//!    单线程的编译器 driver 是目标用法，如实成文。

use crate::ctypes::{c_char, c_int, c_short, c_uint, c_ulonglong};
use crate::errno::{from_libsys, set_errno, EINVAL, ENOMEM, ENOSYS, ENOTSUP};

// ---------- 标志（取值与 glibc/Linux 一致） ----------

pub const POSIX_SPAWN_RESETIDS: c_int = 0x01;
pub const POSIX_SPAWN_SETPGROUP: c_int = 0x02;
pub const POSIX_SPAWN_SETSIGDEF: c_int = 0x04;
pub const POSIX_SPAWN_SETSIGMASK: c_int = 0x08;
pub const POSIX_SPAWN_SETSCHEDPARAM: c_int = 0x10;
pub const POSIX_SPAWN_SETSCHEDULER: c_int = 0x20;

/// file_actions 的单个动作（BORUIX 自有布局，见 `libc/include/spawn.h`）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct spawn_action {
    /// 0 = open，1 = close，2 = dup2
    pub tag: c_int,
    pub fd: c_int,
    pub path: *mut c_char,
    pub oflag: c_int,
    pub mode: c_uint,
    pub newfd: c_int,
    pub _pad: c_int,
}

const NO_ACTION: spawn_action = spawn_action {
    tag: 0,
    fd: 0,
    path: core::ptr::null_mut(),
    oflag: 0,
    mode: 0,
    newfd: 0,
    _pad: 0,
};

/// 动作表容量。**固定、不分配**：`posix_spawn` 常在错误路径上被调用，此时不应再依赖堆。
pub const SPAWN_MAX_ACTIONS: usize = 16;

/// `posix_spawn_file_actions_t`（BORUIX 自有布局；POSIX 只要求它是不透明类型）。
#[repr(C)]
pub struct posix_spawn_file_actions_t {
    pub count: c_int,
    pub _pad: c_int,
    pub actions: [spawn_action; SPAWN_MAX_ACTIONS],
}

/// `posix_spawnattr_t`（BORUIX 自有布局）。
#[repr(C)]
pub struct posix_spawnattr_t {
    pub flags: c_short,
    pub _pad: c_short,
    pub pgroup: c_int,
    pub sigmask: c_ulonglong,
    pub sigdefault: c_ulonglong,
    pub sched_policy: c_int,
    pub sched_priority: c_int,
    pub _reserved: [c_int; 8],
}

/// `posix_spawn_file_actions_init(fa)`：初始化动作表（POSIX 要求可重复 init）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_init(fa: *mut posix_spawn_file_actions_t) -> c_int {
    if fa.is_null() {
        return EINVAL;
    }
    let a = unsafe { &mut *fa };
    a.count = 0;
    a._pad = 0;
    for i in 0..SPAWN_MAX_ACTIONS {
        a.actions[i] = NO_ACTION;
    }
    0
}

/// `posix_spawn_file_actions_destroy(fa)`：销毁动作表（本实现无堆分配，故只是重置）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_destroy(fa: *mut posix_spawn_file_actions_t) -> c_int {
    if fa.is_null() {
        return EINVAL;
    }
    unsafe { posix_spawn_file_actions_init(fa) }
}

unsafe fn fa_push(fa: *mut posix_spawn_file_actions_t, act: spawn_action) -> c_int {
    if fa.is_null() {
        return EINVAL;
    }
    let a = unsafe { &mut *fa };
    let i = a.count as usize;
    if i >= SPAWN_MAX_ACTIONS {
        // 表满**如实报错**（POSIX 允许 ENOMEM）；绝不静默丢弃动作——
        // 丢弃会让调用方以为重定向生效了。
        return crate::errno::ENOMEM;
    }
    a.actions[i] = act;
    a.count += 1;
    0
}

/// `posix_spawn_file_actions_addopen(fa, fd, path, oflag, mode)`：派生时把 `path` 打开到 `fd`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_addopen(
    fa: *mut posix_spawn_file_actions_t,
    fd: c_int,
    path: *const c_char,
    oflag: c_int,
    mode: c_uint,
) -> c_int {
    if fd < 0 || path.is_null() {
        return EINVAL;
    }
    unsafe {
        fa_push(fa, spawn_action { tag: 0, fd, path: path as *mut c_char, oflag, mode, newfd: 0, _pad: 0 })
    }
}

/// `posix_spawn_file_actions_addclose(fa, fd)`：派生时关闭 `fd`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_addclose(
    fa: *mut posix_spawn_file_actions_t,
    fd: c_int,
) -> c_int {
    if fd < 0 {
        return EINVAL;
    }
    unsafe { fa_push(fa, spawn_action { tag: 1, fd, path: core::ptr::null_mut(), oflag: 0, mode: 0, newfd: 0, _pad: 0 }) }
}

/// `posix_spawn_file_actions_adddup2(fa, fd, newfd)`：派生时把 `fd` 复制到 `newfd`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn_file_actions_adddup2(
    fa: *mut posix_spawn_file_actions_t,
    fd: c_int,
    newfd: c_int,
) -> c_int {
    if fd < 0 || newfd < 0 {
        return EINVAL;
    }
    unsafe { fa_push(fa, spawn_action { tag: 2, fd, path: core::ptr::null_mut(), oflag: 0, mode: 0, newfd, _pad: 0 }) }
}

/// `posix_spawnattr_init(attr)`：初始化属性对象（全零 = 无旗标）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_init(attr: *mut posix_spawnattr_t) -> c_int {
    if attr.is_null() {
        return EINVAL;
    }
    let a = unsafe { &mut *attr };
    a.flags = 0;
    a._pad = 0;
    a.pgroup = 0;
    a.sigmask = 0;
    a.sigdefault = 0;
    a.sched_policy = 0;
    a.sched_priority = 0;
    for i in 0..8 {
        a._reserved[i] = 0;
    }
    0
}

/// `posix_spawnattr_destroy(attr)`：销毁属性对象（无堆分配，故只是重置）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_destroy(attr: *mut posix_spawnattr_t) -> c_int {
    unsafe { posix_spawnattr_init(attr) }
}

/// `posix_spawnattr_getflags(attr, flags)`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_getflags(attr: *const posix_spawnattr_t, flags: *mut c_short) -> c_int {
    if attr.is_null() || flags.is_null() {
        return EINVAL;
    }
    unsafe { *flags = (*attr).flags };
    0
}

/// `posix_spawnattr_setflags(attr, flags)`：只接受本系统支持的位（见模块文档第 3 条）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnattr_setflags(attr: *mut posix_spawnattr_t, flags: c_short) -> c_int {
    if attr.is_null() {
        return EINVAL;
    }
    let unsupported = (flags as c_int) & !POSIX_SPAWN_RESETIDS;
    if unsupported != 0 {
        // 如实拒绝：接受但忽略会让调用方以为屏蔽集/进程组生效了。
        return ENOSYS;
    }
    unsafe { (*attr).flags = flags };
    0
}

/// 拼命令行：`argv[1..]` 以空格连接（本 ABI 的 argv[0] 就是整条命令行）。
///
/// 返回 `Err(errno)` 当某个参数含空白（无法无损表达）或过长。
unsafe fn build_cmdline(argv: *const *const c_char, out: &mut [u8; 256]) -> Result<usize, c_int> {
    let mut n = 0usize;
    if argv.is_null() {
        return Ok(0);
    }
    let mut i = 1usize; // 跳过 argv[0]（程序名由 path 参数给出）
    loop {
        let p = unsafe { *argv.add(i) };
        if p.is_null() {
            break;
        }
        let w = unsafe { crate::stdio::cstr_bytes(p) };
        for &b in w.iter() {
            if b == b' ' || b == b'\t' {
                // **不静默切成两个参数**——如实报 EINVAL（见模块文档第 1 条）。
                return Err(EINVAL);
            }
        }
        if i > 1 {
            if n >= out.len() {
                return Err(crate::errno::ERANGE);
            }
            out[n] = b' ';
            n += 1;
        }
        for &b in w.iter() {
            if n >= out.len() {
                return Err(crate::errno::ERANGE);
            }
            out[n] = b;
            n += 1;
        }
        i += 1;
    }
    Ok(n)
}

/// 备份 fd 的基准：备份 fd 一律 ≥ 此值，避开常规 fd 号（见模块文档第 4 条）。
const SAVE_BASE: c_int = 64;

unsafe fn save_fd(orig: &mut [c_int; 32], save: &mut [c_int; 32], n: &mut usize, fd: c_int) -> bool {
    for i in 0..*n {
        if orig[i] == fd {
            return true; // 已备份
        }
    }
    if *n >= 32 {
        return false;
    }
    // fcntl(F_DUPFD, SAVE_BASE)：复制到 ≥ SAVE_BASE 的最低空闲槽；失败 = 原本就关闭。
    let b = unsafe { crate::unistd::fcntl(fd, crate::unistd::F_DUPFD, SAVE_BASE) };
    orig[*n] = fd;
    save[*n] = b; // -1 表示原本是关闭的
    *n += 1;
    true
}

/// `posix_spawn(pid, path, file_actions, attrp, argv, envp)`：派生并运行 `path`（POSIX）。
///
/// 返回 **0** 成功（子进程 pid 写入 `*pid`），否则返回**错误号本身**（POSIX：`posix_spawn`
/// 不返回 -1、不置 errno，而是直接返回错误号）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawn(
    pid: *mut c_int,
    path: *const c_char,
    file_actions: *const posix_spawn_file_actions_t,
    attrp: *const posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    if pid.is_null() || path.is_null() {
        return EINVAL;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => return EINVAL,
    };
    if !attrp.is_null() {
        let f = unsafe { (*attrp).flags } as c_int;
        if f & !POSIX_SPAWN_RESETIDS != 0 {
            return ENOSYS;
        }
    }
    let _ = envp; // 见模块文档第 2 条：本系统的环境由内核重建，envp 无法转交。
    let mut cmd = [0u8; 256];
    let n = match unsafe { build_cmdline(argv, &mut cmd) } {
        Ok(n) => n,
        Err(e) => return e,
    };
    // ---- file_actions：先全部备份，再施加，派生后逆序还原 ----
    let mut orig: [c_int; 32] = [-1; 32];
    let mut save: [c_int; 32] = [-1; 32];
    let mut nsaved = 0usize;
    let mut applied = false;
    if !file_actions.is_null() {
        let fa = unsafe { &*file_actions };
        let cnt = fa.count as usize;
        // 预扫：把每个将被触及的 fd 先备份好（备份 fd ≥ 64，不会与目标 fd 撞号）。
        for k in 0..cnt {
            let a = &fa.actions[k];
            let target = if a.tag == 2 { a.newfd } else { a.fd };
            if !unsafe { save_fd(&mut orig, &mut save, &mut nsaved, target) } {
                unsafe { restore_fds(&orig, &save, nsaved) };
                return ENOMEM;
            }
        }
        // 施加（按登记顺序，POSIX 语义）。
        for k in 0..cnt {
            let a = &fa.actions[k];
            let rc = unsafe {
                match a.tag {
                    0 => {
                        let nf = crate::unistd::open(a.path, a.oflag, a.mode);
                        if nf < 0 {
                            -1
                        } else {
                            let r = crate::unistd::dup2(nf, a.fd);
                            let _ = crate::unistd::close(nf);
                            r
                        }
                    }
                    1 => crate::unistd::close(a.fd),
                    _ => crate::unistd::dup2(a.fd, a.newfd),
                }
            };
            if rc < 0 {
                let e = crate::errno::errno();
                unsafe { restore_fds(&orig, &save, nsaved) };
                return if e > 0 { e } else { EINVAL };
            }
            applied = true;
        }
    }
    let _ = applied;
    let r = libsys::exec_path(p, &cmd[..n]);
    if !file_actions.is_null() {
        unsafe { restore_fds(&orig, &save, nsaved) };
    }
    match r {
        Ok(child) => {
            unsafe { *pid = child as c_int };
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            from_libsys(e)
        }
    }
}

unsafe fn restore_fds(orig: &[c_int; 32], save: &[c_int; 32], n: usize) {
    // 逆序还原（后施加的先撤）。
    let mut i = n;
    while i > 0 {
        i -= 1;
        unsafe {
            if save[i] >= 0 {
                let _ = crate::unistd::dup2(save[i], orig[i]);
                let _ = crate::unistd::close(save[i]);
            } else {
                // 原本就是关闭的：关掉（不能「还原成关闭」之外的状态）。
                let _ = crate::unistd::close(orig[i]);
            }
        }
    }
}

/// `posix_spawnp(pid, file, file_actions, attrp, argv, envp)`：同 `posix_spawn`，但按 `PATH` 搜索。
///
/// **诚实边界**：`file` 含 `/` 时直接用它（不做搜索，与 POSIX 一致）。`PATH` 取自本进程环境；
/// **未设 `PATH` 时如实返回 `ENOENT`**（不内置默认——本系统的默认 `PATH` 定义在 **shell** 仓，
/// libc 复制一份就是 S15 的漂移源，与 `confstr` 判不支持同因）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_spawnp(
    pid: *mut c_int,
    file: *const c_char,
    file_actions: *const posix_spawn_file_actions_t,
    attrp: *const posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    if file.is_null() {
        return EINVAL;
    }
    let f = unsafe { crate::stdio::cstr_bytes(file) };
    if f.contains(&b'/') {
        return unsafe { posix_spawn(pid, file, file_actions, attrp, argv, envp) };
    }
    let pathv = unsafe { crate::stdlib::getenv(b"PATH\0".as_ptr() as *const c_char) };
    if pathv.is_null() {
        return crate::errno::ENOENT;
    }
    let path = unsafe { crate::stdio::cstr_bytes(pathv) };
    for dir in path.split(|&b| b == b':') {
        let mut buf = [0u8; 256];
        let mut n = 0usize;
        let d: &[u8] = if dir.is_empty() { b"." } else { dir };
        if d.len() + 1 + f.len() + 1 > buf.len() {
            continue;
        }
        buf[..d.len()].copy_from_slice(d);
        n += d.len();
        buf[n] = b'/';
        n += 1;
        buf[n..n + f.len()].copy_from_slice(f);
        n += f.len();
        buf[n] = 0;
        let rc = unsafe {
            posix_spawn(
                pid,
                buf.as_ptr() as *const c_char,
                file_actions,
                attrp,
                argv,
                envp,
            )
        };
        if rc == 0 {
            return 0;
        }
        if rc != crate::errno::ENOENT {
            return rc;
        }
    }
    crate::errno::ENOENT
}
