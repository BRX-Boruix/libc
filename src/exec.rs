//! `exec*` 家族（3P4-12 起**真的替换映像**）。
//!
//! 本系统从 3P4-12 起有 `SYS_TASK_EXEC`（0x30）：成功时当前进程的映像被换掉、从新
//! 入口继续执行（**pid 不变**）；失败时返回 -1 并置 errno，**旧映像继续运行**。
//!
//! **argv 的传递形态**：本 ABI 的命令行是**单个字符串**（`argc=1`，`argv[0]` 就是整条
//! 命令行），与 `posix_spawn` 同一约定，故 `execv` 复用同一个拼接器
//! （[`crate::spawn::build_cmdline`]）：`argv[0]` 由 `path` 承担、其余以空格连接；
//! 含空白的元素**如实 EINVAL** 而不是静默切成两个参数。

use crate::ctypes::{c_char, c_int};
use crate::errno::{set_errno, EINVAL, ENOTSUP};

/// `system(command)`：交给 shell 执行。
///
/// **诚实边界**：本系统**没有 `/bin/sh` 的约定**——`system` 需要「起 shell + 等它结束」，
/// 而 shell 程序在 `/programs/shell.elf`，把这条路径硬编码进 libc 就是 S01 的硬编码环境。
/// 故**如实返回 -1 并置 ENOTSUP**——提供符号是为了让 C++ 标准库（`<cstdlib>` 需要
/// `system` 的声明）能编译链接，与 `ttyname` 同一处理原则（S09：宁可报错，绝不假装成功）。
///
/// **3P4-12 之后这条边界只剩一半**：exec 已经能替换映像了，缺的纯粹是"shell 在哪"
/// 这条部署事实——它该由调用方或 shell 仓决定，不该由 libc 猜。
/// `command == NULL` 时按 POSIX 返回「有 shell 可用」的非零值（本系统**有** shell 程序）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn system(command: *const c_char) -> c_int {
    if command.is_null() {
        return 1; // POSIX：非零 = 有 shell 可用。本系统确有 shell（只是路径不由 libc 决定）。
    }
    set_errno(ENOTSUP);
    -1
}

/// `execv(path, argv)`：用 `path` 的映像**替换当前进程**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(path: *const c_char, argv: *const *const c_char) -> c_int {
    unsafe { exec_impl(path, argv, core::ptr::null()) }
}

/// `execve(path, argv, envp)`：同上；`envp` 的边界见 [`exec_impl`]。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    unsafe { exec_impl(path, argv, envp) }
}

/// `execv` / `execve` 的共同实现（两者的差别只有 `envp`）。
///
/// **诚实边界（3P4-2b）**：本系统的环境由**内核按权威状态合成**（`PATH` 单源，ADR-028），
/// `envp` **目前仍无法转交**——裁定已下、实现未做。这里忽略它而不是拒绝：POSIX 程序普遍传
/// `environ`，拒绝会让 `execve` 完全不可用；但这一点必须成文，不能假装生效。
///
/// 裁定（owner，2026-10-18）：**接受显式 `envp`，但保留键拒绝**——`PATH`/`PWD` 由内核权威单源，
/// 调用方设了就 `EINVAL`；其余键允许。落地在 `3P4-2b`。
unsafe fn exec_impl(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let _ = envp;
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut cmd = [0u8; 256];
    let n = match unsafe { crate::spawn::build_cmdline(argv, &mut cmd) } {
        Ok(n) => n,
        Err(e) => {
            set_errno(e);
            return -1;
        }
    };
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_errno(EINVAL);
            return -1;
        }
    };
    match libsys::exec_replace(p, &cmd[..n]) {
        // 成功路径不返回（映像已换，内核直接从新入口继续）；`Infallible` 让这一点
        // 由类型系统强制——写不出"成功了却继续往下走"的代码。
        Ok(never) => match never {},
        Err(e) => {
            set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}
