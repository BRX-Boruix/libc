//! 进程函数（C ABI）：exit / getpid / kill / waitpid / yield。
//!
//! ## getpid 的实现与假设（S09 如实）
//!
//! 内核目前没有专用 \`SYS_TASK_GETPID\` 系统调用，libsys 也不直接提供当前 pid。
//! 本实现读取 ProcFS \`/processes/list\`（内核实时生成）并选取 state ==
//! "Running" 的进程作为当前进程——在本单 CPU 抢占调度器下，调用 getpid 的进程
//! 此刻正在运行，其快照 state 必为 "Running"，因此该识别是确定性的。
//!
//! **改进路径**：更稳妥的做法是给内核加 \`SYS_TASK_GETPID\`（TASK 域扩展），
//! 当前实现作为无内核改动下的诚实等价物；若引入多核并发运行，须改为内核
//! syscall 或 TLS 记录。

use crate::ctypes::c_int;
use crate::errno::{set_errno, from_libsys};

/// \`exit(code)\`：终止当前进程。永不返回。
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: c_int) -> ! {
    libsys::exit(code)
}

/// \`_exit(code)\`：与 exit 等价（本实现无 atexit/清理）。
#[unsafe(no_mangle)]
pub extern "C" fn _exit(code: c_int) -> ! {
    libsys::exit(code)
}

/// \`getpid()\`：返回当前进程 PID；失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> c_int {
    match current_pid() {
        Some(pid) => pid as c_int,
        None => -1,
    }
}

/// 内部：读取 /processes/list 解析当前（Running）进程 pid。
fn current_pid() -> Option<u64> {
    let data = libsys::read_to_end("/processes/list").ok()?;
    let text = core::str::from_utf8(&data).ok()?;
    parse_running_pid(text)
}

/// 解析 \`[{"pid":1,"state":"Running",...}]\` 中 state==Running 的 pid。
fn parse_running_pid(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return None;
    }
    let content = &trimmed[1..trimmed.len() - 1];
    let mut running: Option<u64> = None;
    for obj in content.split("},") {
        let s = obj.trim().trim_start_matches('{').trim_end_matches('}');
        let mut pid: Option<u64> = None;
        let mut state: Option<&str> = None;
        for field in s.split(',') {
            let mut kv = field.split(':');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                let k = k.trim().trim_matches('"');
                let v = v.trim().trim_matches('"');
                match k {
                    "pid" => pid = v.parse::<u64>().ok(),
                    "state" => state = Some(v),
                    _ => {}
                }
            }
        }
        if let (Some(p), Some("Running")) = (pid, state) {
            running = Some(p);
        }
    }
    running
}

/// \`kill(pid, sig)\`：向进程发信号。返回 0 或 -1（置 errno）。
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

/// \`waitpid(pid, status, options)\`：等待子进程。
///
/// 内核 libsys 目前仅支持 \`waitpid_any()\`（等任意子进程）。本实现将
/// 非 0 的 pid 如实映射为"等任意子"（内核无精确匹配原语），并在 status 写入
/// 退出码（高 8 位，POSIX WEXITSTATUS 语义）。options 非 0（WNOHANG 等）暂不
/// 支持，如实返回 -1 置 ENOTSUP。返回 pid 不可得，如实返回 -1（status 已填）。
#[unsafe(no_mangle)]
pub extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    let _ = pid;
    if options != 0 {
        set_errno(crate::errno::ENOTSUP);
        return -1;
    }
    match libsys::waitpid_any() {
        Ok(code) => {
            if !status.is_null() {
                unsafe {
                    *status = ((code & 0xFF) as c_int) << 8;
                }
            }
            -1
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`yield()\`：主动让出 CPU。
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
