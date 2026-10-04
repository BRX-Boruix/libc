//! 内存映射（3P4-4）：`mmap` 的 C 出口与 PROT_*/MAP_* 常量。
//!
//! 编号与内核 mm 层、libsys **同一事实**（三处互指；单点定义处是内核
//! `mm::user_space`，因为 W^X 策略在那里裁决）。

use crate::ctypes::{c_int, c_void, off_t, size_t};
use crate::errno::{from_libsys, set_errno, ENOTSUP};

pub const PROT_READ: c_int = 1;
pub const PROT_WRITE: c_int = 2;
pub const PROT_EXEC: c_int = 4;
pub const MAP_SHARED: c_int = 0x01;
pub const MAP_PRIVATE: c_int = 0x02;
pub const MAP_ANONYMOUS: c_int = 0x20;
/// POSIX 失败哨兵（`(void *)-1`）。
pub const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

/// `mprotect(addr, len, prot)`：修改已映射内存权限（3P4-5）。
///
/// W^X 由内核**单点**拒绝（写+执行同页 → EINVAL），本层不预检。`prot == 0`
/// （PROT_NONE）当前如实 `ENOTSUP`——抽象层没有「存在但不可访问」的权限表示。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mprotect(addr: *mut c_void, len: size_t, prot: c_int) -> c_int {
    let prot_bits = (prot as u64) & 0x7;
    match libsys::mprotect(addr as u64, len as u64, prot_bits) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `mmap(addr, length, prot, flags, fd, offset)`：匿名映射（3P4-4）。
///
/// **当前只支持匿名映射**：`addr` 必须为 NULL（由内核选地址）、`flags` 必须含
/// `MAP_ANONYMOUS` 且不含 `MAP_SHARED`（共享映射走 libsys 的 shm_* 通道），
/// `fd`/`offset` 忽略。其余组合如实 `ENOTSUP`——绝不假装成功（文件映射需要另一条
/// 通道，尚未实现）。
///
/// W^X 由内核**单点**拒绝（写+执行同页 → `EINVAL`），本层不预检（S13：策略不两处漂移）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(
    addr: *mut c_void,
    length: size_t,
    prot: c_int,
    flags: c_int,
    _fd: c_int,
    _offset: off_t,
) -> *mut c_void {
    if !addr.is_null() || flags & MAP_ANONYMOUS == 0 || flags & MAP_SHARED != 0 {
        set_errno(ENOTSUP);
        return MAP_FAILED;
    }
    // 只透传三个权限位；其余位由内核如实拒绝（未知位不静默忽略）。
    let prot_bits = (prot as u64) & 0x7;
    match libsys::mmap_prot(length as u64, prot_bits) {
        Ok(va) => va as *mut c_void,
        Err(e) => {
            set_errno(from_libsys(e));
            MAP_FAILED
        }
    }
}
