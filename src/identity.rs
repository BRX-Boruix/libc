//! 身份与能力的 C ABI 出口（boruix 专有——POSIX 的 `setuid` **表达不了**能力轴）。
//!
//! **为什么需要它**：本内核的身份是 `(uid, gid, caps)` **三元组**，而 POSIX 的 `setuid`/`setgid`
//! 只动前两维、**保留 caps**（见 `posix_batch3.rs` 的 `setuid`）。于是「**主动降权**」这件事
//! 在 C ABI 上**没有出口**：`setuid(1000)` 之后 `CAP_OWNER` 仍在，`check_access` 第 2 步照样
//! 直接放行——实测（`mode_umask_check.c`）里表现为「只读文件仍可 O_WRONLY」。
//!
//! **为什么这件事重要（不是测试专用）**：自动化通道（`--selftest` / `--selftest-quick` /
//! `--run=`）**故意**跑在 login 降权**之前**（`init/src/main.rs:1170-1172` 成文），故它们
//! 持 init 的 `system()` 全能力。任何**验证权限行为**的程序若走这些通道，都会看到一个
//! 特权进程——我因此连错两次结论。给它一条显式降权的路，这类验证才可写。

use core::ffi::c_int;

use crate::ctypes::c_void;
use crate::errno::{from_libsys, set_errno, EINVAL};

/// `boruix_identity_set(uid, gid, caps)`：把调用进程组的身份设为三元组（**含能力位**）。
///
/// 能力位（与内核 `task::Caps` 对齐，ADR-040 §2.3）：`SYSTEM=1<<0`、`DEVICE=1<<1`、
/// `MEMORY=1<<2`、`KILL=1<<3`、`OWNER=1<<4`。**传 0 即彻底降权**（这是本函数存在的主要用途）。
///
/// 授权：无 `CAP_SYSTEM` 时**只能改 caps、不得改 uid/gid**（A2-1 规则，内核侧单点）。
/// 成功返回 0，失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boruix_identity_set(uid: u32, gid: u32, caps: u32) -> c_int {
    match libsys::identity_set(uid, gid, caps) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `boruix_identity_query(out)`：把调用进程的**真实** `(uid, gid, caps)` 写入 `out`。
///
/// `out` 指向 12 字节可写缓冲，布局 = 3 个 u32：`[uid, gid, caps]`（与 `libsys::IdentityInfo` 同布局）。
/// 成功返回 0，失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boruix_identity_query(out: *mut c_void) -> c_int {
    if out.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match libsys::identity_query() {
        Ok(info) => {
            let p = out as *mut u32;
            unsafe {
                *p = info.uid;
                *p.add(1) = info.gid;
                *p.add(2) = info.caps;
            }
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}