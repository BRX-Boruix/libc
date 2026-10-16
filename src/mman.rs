//! 内存映射（3P4-4）：`mmap` 的 C 出口与 PROT_*/MAP_* 常量。
//!
//! 编号与内核 mm 层、libsys **同一事实**（三处互指；单点定义处是内核
//! `mm::user_space`，因为 W^X 策略在那里裁决）。

use crate::ctypes::{c_int, c_void, off_t, size_t};
use crate::errno::{from_libsys, set_errno, EINVAL, ENOTSUP};

pub use crate::abi_generated::PROT_READ;
pub use crate::abi_generated::PROT_WRITE;
pub use crate::abi_generated::PROT_EXEC;
pub use crate::abi_generated::MAP_SHARED;
pub use crate::abi_generated::MAP_PRIVATE;
pub use crate::abi_generated::MAP_ANONYMOUS;
/// POSIX 失败哨兵（`(void *)-1`）。
pub use crate::abi_generated::MAP_FAILED;

/// `boruix_memory_query(addr, out)`：查询 `addr` 所在 4KB 页的**页表真值**（KM2 / 0x22）。
///
/// 成功返回 0 并把 u64 位图写入 `out`（须指向 8 字节可写缓冲）；失败返回 -1 置 errno。
/// 位图 `0` = 未映射（含 demand 区已声明但未触碰——查询**只读页表，绝不触发补页**）；
/// 否则为 `MEMQ_PRESENT | [MEMQ_USER] | [MEMQ_WRITABLE]`。
///
/// **为什么补这个出口**：内核 `sys_memory_query` 与 libsys 的 `SYS_MEMORY_QUERY`/`MEMQ_*`
/// 早就在，但 libc 没有 C ABI 出口 ⇒ 用户程序只能自己写 `int 0x80`。那是**可达性缺口**，
/// 不是缺功能（本仓的夹具当时就是这么绕过它的）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn boruix_memory_query(addr: *const c_void, out: *mut u64) -> c_int {
    if out.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match libsys::memory_query(addr as u64) {
        Ok(bits) => {
            unsafe { *out = bits };
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `munmap(addr, len)`：解除映射（`mmap` 的配对操作）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut c_void, len: size_t) -> c_int {
    match libsys::munmap(addr as u64, len as u64) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `mprotect(addr, len, prot)`：修改已映射内存权限（3P4-5）。
///
/// W^X 由内核**单点**拒绝（写+执行同页 → EINVAL），本层不预检。
/// `prot == 0`（POSIX **PROT_NONE**）**已支持**（语义忠实性审查登记项⑥）：映射保留为
/// present 但**不带 user 位** ⇒ 任何访问触发保护性 #PF（SIGSEGV），映射仍在册（可再改回、可 munmap）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mprotect(addr: *mut c_void, len: size_t, prot: c_int) -> c_int {
    // 同 mmap：不掩码，未知位由内核单点拒绝（`map_flags_from_prot` ⇒ InvalidParam ⇒ EINVAL）。
    match libsys::mprotect(addr as u64, len as u64, prot as u64) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `mmap(addr, length, prot, flags, fd, offset)`：匿名映射（3P4-4）。
///
/// **当前只支持匿名私有映射**：`flags` 必须含 `MAP_ANONYMOUS`、不含 `MAP_SHARED`
/// （共享映射走 libsys 的 shm_* 通道），`fd`/`offset` 忽略。其余组合如实 `ENOTSUP`
/// ——绝不假装成功（文件映射需要另一条通道，尚未实现）。
///
/// `addr` 按 POSIX 是**提示**：非 NULL 且未设 `MAP_FIXED` 时实现可以另择地址，调用方
/// **必须**用返回值。故本实现**忽略**该提示（由内核选地址），**不**因此拒绝调用；
/// `MAP_FIXED` 本身则**如实拒绝**（做不到「必须放在这里」），见下方注释。
///
/// W^X 由内核**单点**拒绝（写+执行同页 → `EINVAL`），本层不预检（S13：策略不两处漂移）。
///
/// `prot == 0` 是 POSIX 的 **PROT_NONE**（登记项⑥）：返回的映射**存在但不可访问**
/// （任何访问 → SIGSEGV）。要「默认 RW」请显式传 `PROT_READ | PROT_WRITE`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(
    addr: *mut c_void,
    length: size_t,
    prot: c_int,
    flags: c_int,
    _fd: c_int,
    _offset: off_t,
) -> *mut c_void {
    // **已知标志位** = POSIX 的 MAP_SHARED | MAP_PRIVATE | MAP_ANONYMOUS。其余位如实拒绝：
    // 尤其 MAP_FIXED（Linux 值 0x10）——本实现做不到「必须放在指定地址」，而此前它落在
    // 未被检查的位里被**静默忽略**：调用方以为映射落在 `addr` 上，实际落在内核挑的地址上。
    // POSIX 里 MAP_FIXED **不是**提示，静默忽略等于伪造。
    let known = MAP_SHARED | MAP_PRIVATE | MAP_ANONYMOUS;
    if flags & !known != 0 || flags & MAP_ANONYMOUS == 0 || flags & MAP_SHARED != 0 {
        set_errno(ENOTSUP);
        return MAP_FAILED;
    }
    // `addr` 只是提示（见函数文档）：忽略它，由内核选地址。
    let _ = addr;
    // **不在这里掩码**：未知 prot 位的拒绝是内核的**单点**——`mm::user_space::map_flags_from_prot`
    // 对 `prot & !(READ|WRITE|EXEC) != 0` 返回 InvalidParam（其文档明确写「未知位同样不静默忽略」）。
    // 此前写成 `& 0x7`，把未知位**静默丢掉**：`mmap(..., prot = 0x100, ...)` 会被当成
    // `prot = 0`（内核的 PROT_DEFAULT = RW）**成功**返回一段可写内存，而调用方以为自己的
    // 权限要求被满足了。语义忠实性审查第六轮改。
    match libsys::mmap_prot(length as u64, prot as u64) {
        Ok(va) => va as *mut c_void,
        Err(e) => {
            set_errno(from_libsys(e));
            MAP_FAILED
        }
    }
}
