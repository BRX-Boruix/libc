//! `exec*` 家族中**无法忠实实现**的成员（3P6-2 已判 B 类）。
//!
//! **为什么还要提供符号**：本系统没有「替换当前进程映像」的 syscall（只有派生），故 `execv`/`execve`
//! 无法忠实实现。但**引用它们的代码要能编译/链接**——libgcc 的 `libgcov-interface.c` 就用了它们
//! （第 43 轮实测：`implicit declaration of function 'execv'`）。
//!
//! **语义（S09 诚实）**：调用即返回 -1 并置 `ENOTSUP`——**绝不假装成功**。调用方会得到一个明确的
//! 错误，而不是静默地什么都没发生。`execvp` 已在 `unistd.rs` 里按同一原则实现。

use crate::ctypes::{c_char, c_int};
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
