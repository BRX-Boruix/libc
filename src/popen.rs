//! `system` / `popen` / `pclose` —— 需要 shell 的三个函数（3P4-14，2026-10-18 裁定）。
//!
//! **shell 路径的唯一来源是环境变量 `SHELL`**：libc 不硬编码任何路径——`/programs/shell.elf`
//! 是**部署事实**不是契约（ADR-055）。谁导出它？**shell 自己**在启动时
//! `setenv("SHELL", <自己的 execfn>)`；`execfn` 来自内核的 `AT_EXECFN`，故连「shell 装在哪」
//! 都不需要写死。
//!
//! 缺失 `SHELL` 时：`system(cmd)` 返回 -1 + `ENOTSUP`，`popen` 返回 NULL + `ENOENT`。
//! **不内置默认**——与 `execvp` 处理 `PATH` 同一条纪律（不猜部署事实）。
//!
//! **调用约定**：本系统的 shell 非交互模式是「有 argv[0] 就执行它然后退出」，而本 ABI 的
//! `argv[0]` 就是**整条命令行**（ABI §4）。故 `system("echo hi")` = `$SHELL "echo hi"`，
//! **不是** `sh -c "echo hi"`。

use crate::ctypes::{c_char, c_int};
use crate::errno::{set_errno, EAGAIN, EINVAL, ENOENT, ENOMEM, ENOTSUP};

/// 读 `SHELL` 环境变量；未设时返回 `None`（调用方如实失败，不猜默认值）。
fn shell_path() -> Option<&'static [u8]> {
    let p = unsafe { crate::stdlib::getenv(b"SHELL\0".as_ptr() as *const c_char) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { crate::stdio::cstr_bytes(p) })
}

/// 按 `EAGAIN` 重试收尸，返回 POSIX 编码的等待状态（`code << 8`）。
///
/// 本系统的**阻塞** `waitpid` 在「子进程尚未退出、且当下无法切换」时如实返回 `EAGAIN`
/// （内核「无可切对象」的诚实回报），调用方须重试——shell 的 acee2e/trave2e 是同一纪律。
fn reap(pid: c_int) -> c_int {
    let mut st: c_int = 0;
    loop {
        let w = crate::process::waitpid(pid, &mut st, 0);
        if w == pid {
            return st;
        }
        if w == -1 && crate::errno::errno() == EAGAIN {
            let _ = libsys::yield_now();
            continue;
        }
        return -1;
    }
}

/// `system(command)`：把整条命令行交给 `$SHELL` 执行并等它结束。
///
/// 返回值是 **`waitpid` 的原始状态**（POSIX：调用方用 `WEXITSTATUS` 解码），不在这里
/// 替调用方解码。`command == NULL` 时按 POSIX 返回「有命令处理器可用」的非零值。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn system(command: *const c_char) -> c_int {
    if command.is_null() {
        // POSIX：非零 = 有命令处理器。本系统确有 shell，只是路径由环境给出。
        return 1;
    }
    let Some(shell) = shell_path() else {
        set_errno(ENOTSUP);
        return -1;
    };
    let Ok(shell_str) = core::str::from_utf8(shell) else {
        set_errno(EINVAL);
        return -1;
    };
    let cmd = unsafe { crate::stdio::cstr_bytes(command) };
    let pid = crate::process::fork();
    if pid < 0 {
        return -1;
    }
    if pid == 0 {
        // 子进程：**替换**自身映像为 shell（不是再派生一次）。
        let _ = libsys::exec_replace(shell_str, cmd);
        crate::process::_exit(127);
    }
    reap(pid)
}

/// `popen(command, type)`：起一个 `$SHELL` 子进程，把它的 stdout（`"r"`）或 stdin（`"w"`）
/// 接到一条管道上，返回包住本端的 `FILE *`。
///
/// `type` 只接受 `"r"` / `"w"`：`"e"`（cloexec）**如实 `EINVAL`**——本系统没有「fork 与 exec
/// 之间设 cloexec」的窗口，真做了也只是语义装饰，不如说清做不到。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn popen(command: *const c_char, mode: *const c_char) -> *mut crate::stdio::FILE {
    if command.is_null() || mode.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let m = unsafe { crate::stdio::cstr_bytes(mode) };
    // 只接受单字符 "r"/"w"：多字符（"re"/"we"）如实拒绝，不静默按首字符处理。
    if m.len() != 1 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let reading = match m[0] {
        b'r' => true,
        b'w' => false,
        _ => {
            set_errno(EINVAL);
            return core::ptr::null_mut();
        }
    };
    let Some(shell) = shell_path() else {
        set_errno(ENOENT);
        return core::ptr::null_mut();
    };
    let Ok(shell_str) = core::str::from_utf8(shell) else {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    };
    let cmd = unsafe { crate::stdio::cstr_bytes(command) };

    let mut fds: [c_int; 2] = [-1, -1];
    if crate::posix_batch2::pipe(fds.as_mut_ptr()) != 0 {
        return core::ptr::null_mut();
    }
    let pid = crate::process::fork();
    if pid < 0 {
        let _ = crate::unistd::close(fds[0]);
        let _ = crate::unistd::close(fds[1]);
        return core::ptr::null_mut();
    }
    if pid == 0 {
        // 子进程：把本端接到标准流，另一端关掉（否则父进程永远读不到 EOF）。
        if reading {
            let _ = crate::unistd::close(fds[0]);
            let _ = crate::unistd::dup2(fds[1], 1);
            let _ = crate::unistd::close(fds[1]);
        } else {
            let _ = crate::unistd::close(fds[1]);
            let _ = crate::unistd::dup2(fds[0], 0);
            let _ = crate::unistd::close(fds[0]);
        }
        let _ = libsys::exec_replace(shell_str, cmd);
        crate::process::_exit(127);
    }
    // 父进程：关掉不用的一端，本端包成 FILE 并记下要收尸的 pid。
    let keep = if reading {
        let _ = crate::unistd::close(fds[1]);
        fds[0]
    } else {
        let _ = crate::unistd::close(fds[0]);
        fds[1]
    };
    let f = unsafe { crate::stdio::fdopen(keep, mode) };
    if f.is_null() {
        let _ = crate::unistd::close(keep);
        // 子进程已在跑：必须收尸，否则留一个僵尸。
        let _ = reap(pid);
        set_errno(ENOMEM);
        return core::ptr::null_mut();
    }
    unsafe { (*f).pclose_pid = pid };
    f
}

/// `pclose(stream)`：关闭 `popen` 流并等子进程结束，返回其 `waitpid` 状态。
///
/// **先 `fclose` 再收尸**：不关本端就等，子进程可能永远等不到 EOF（写方向尤其明显）。
/// `fclose` 失败时仍要收尸——否则泄漏一个僵尸。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pclose(stream: *mut crate::stdio::FILE) -> c_int {
    if stream.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let pid = unsafe { (*stream).pclose_pid };
    if pid <= 0 {
        // 不是 popen 流：POSIX 对此未定义；如实报 EINVAL，不猜。
        set_errno(EINVAL);
        return -1;
    }
    unsafe { (*stream).pclose_pid = 0 };
    let _ = unsafe { crate::stdio::fclose(stream) };
    reap(pid)
}
