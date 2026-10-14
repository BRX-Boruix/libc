//! 调度优先级常量（POSIX `<sys/resource.h>` 的子集）。
//!
//! **诚实边界**：本系统无进程组、无多用户（ADR-043）⇒ 只实现 `PRIO_PROCESS`。
//! `PRIO_PGRP`/`PRIO_USER` 的取值仍如实定义（POSIX 规定），但 `getpriority`/`setpriority`
//! 对它们返回 -1 置 `EINVAL`——不假装支持。

use crate::ctypes::c_int;

/// `PRIO_PROCESS`：作用于单个进程（POSIX 取值 0）。
pub use crate::abi_generated::PRIO_PROCESS;
/// `PRIO_PGRP`：作用于进程组（本系统无进程组，如实不支持）。
pub use crate::abi_generated::PRIO_PGRP;
/// `PRIO_USER`：作用于某用户的所有进程（本系统无多用户，如实不支持）。
pub use crate::abi_generated::PRIO_USER;

/// POSIX 的 `priority = 20 - nice` 里的基数（`getpriority`/`setpriority` 共用）。
pub const PRIORITY_BASE: c_int = 20;
