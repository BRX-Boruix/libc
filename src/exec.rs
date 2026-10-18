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
use crate::errno::{set_errno, EINVAL};

/// 环境里由**内核权威单源**提供的键（与内核 `RESERVED_ENV_KEYS` 同源，S15）。
///
/// 内核会**拒绝**这几个键（`EINVAL`），故本层必须先把它们滤掉——否则
/// `execve(path, argv, environ)` 恒失败（`environ` 里本来就有 `PATH`），`execve` 就成了
/// 不可用的符号。滤掉**不等于**丢弃：子进程拿到的 `PATH`/`PWD` 是内核给的那份。
const RESERVED_ENV_KEYS: [&[u8]; 2] = [b"PATH", b"PWD"];

/// 从 `char *const envp[]` 收集环境条目，**滤掉保留键**。
///
/// 缺 `=` 的畸形条目**原样传下去**：内核会如实 `EINVAL`，比在库里悄悄跳过更诚实
/// （静默丢弃会让调用方以为它生效了）。
unsafe fn collect_envp(envp: *const *const c_char) -> alloc::vec::Vec<alloc::vec::Vec<u8>> {
    let mut out: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    if envp.is_null() {
        return out;
    }
    let mut i = 0usize;
    loop {
        let p = unsafe { *envp.add(i) };
        if p.is_null() {
            break;
        }
        i += 1;
        let s = unsafe { crate::stdio::cstr_bytes(p) };
        let reserved = match s.iter().position(|&b| b == b'=') {
            Some(k) => RESERVED_ENV_KEYS.iter().any(|r| *r == &s[..k]),
            None => false,
        };
        if reserved {
            continue;
        }
        let mut v: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        v.extend_from_slice(s);
        out.push(v);
    }
    out
}

/// `execv(path, argv)`：用 `path` 的映像**替换当前进程**，环境沿用当前 `environ`
/// （POSIX：`execv` 与 `execve` 的差别就是环境来源）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execv(path: *const c_char, argv: *const *const c_char) -> c_int {
    let env = unsafe { *core::ptr::addr_of!(crate::stdlib::environ) };
    unsafe { exec_impl(path, argv, env as *const *const c_char) }
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
/// **`envp` 的语义（3P4-2b，2026-10-18 裁定）**：显式环境**接受**，与内核合成的权威引导
/// 环境（`PATH`/`PWD`）合并后交付给新映像。`PATH`/`PWD` 由内核单源提供（ADR-028），
/// 本层在 [`collect_envp`] 里**滤掉**它们（内核遇到会 `EINVAL`）。
/// `envp == NULL` ⇒ 不传显式环境，子进程只拿权威引导环境（与 `execv` 之前的行为一致）。
unsafe fn exec_impl(
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
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
    // 环境：先收集（滤掉保留键），再交给内核与权威引导环境合并。
    let env = unsafe { collect_envp(envp) };
    let mut refs: alloc::vec::Vec<&[u8]> = alloc::vec::Vec::new();
    for e in env.iter() {
        refs.push(e.as_slice());
    }
    match libsys::exec_replace_env(p, &cmd[..n], &refs) {
        // 成功路径不返回（映像已换，内核直接从新入口继续）；`Infallible` 让这一点
        // 由类型系统强制——写不出"成功了却继续往下走"的代码。
        Ok(never) => match never {},
        Err(e) => {
            set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}
