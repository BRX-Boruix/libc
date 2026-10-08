//! `exec*` 家族中**无法忠实实现**的成员（3P6-2 已判 B 类）。
//!
//! **为什么还要提供符号**：本系统没有「替换当前进程映像」的 syscall（只有派生），故 `execv`/`execve`
//! 无法忠实实现。但**引用它们的代码要能编译/链接**——libgcc 的 `libgcov-interface.c` 就用了它们
//! （第 43 轮实测：`implicit declaration of function 'execv'`）。
//!
//! **语义（S09 诚实）**：调用即返回 -1 并置 `ENOTSUP`——**绝不假装成功**。调用方会得到一个明确的
//! 错误，而不是静默地什么都没发生。`execvp` 已在 `unistd.rs` 里按同一原则实现。

use crate::ctypes::{c_char, c_int};

/// `system(command)`：交给 shell 执行。
///
/// **诚实边界**：本系统**没有 `/bin/sh` 的约定**，且 `system` 需要「起 shell + 等它结束」，
/// 而本 libc 的 `posix_spawn` 只能按**绝对路径**起程序（shell 在 `/programs/shell.elf`，
/// 但把这条路径硬编码进 libc 就是 S01 的硬编码环境）。故**如实返回 -1 并置 ENOTSUP**——
/// 提供符号是为了让 C++ 标准库（`<cstdlib>` 需要 `system` 的声明）能编译链接，
/// 与 `execv`/`execve` 同一处理原则（S09：宁可报错，绝不假装成功）。
/// `command == NULL` 时按 POSIX 返回「有 shell 可用」的非零值（本系统**有** shell 程序）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn system(command: *const c_char) -> c_int {
    if command.is_null() {
        return 1; // POSIX：非零 = 有 shell 可用。本系统确有 shell（只是路径不由 libc 决定）。
    }
    set_errno(ENOTSUP);
    -1
}
use crate::errno::{set_errno, ENOTSUP};

/// `execv(path, argv)`：本系统无替换映像的 syscall ⇒ 如实 `ENOTSUP`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(_path: *const c_char, _argv: *const *const c_char) -> c_int {
    set_errno(ENOTSUP);
    -1
}

/// `execve(path, argv, envp)`：同上；且本系统环境由内核重建（3P4-2b），`envp` 亦无法转交。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execve(
    _path: *const c_char,
    _argv: *const *const c_char,
    _envp: *const *const c_char,
) -> c_int {
    set_errno(ENOTSUP);
    -1
}
