//! C++ 标准库所需的补充项（libstdc++ 编译驱动，逐项都有真实语义或成文的诚实边界）。

use crate::ctypes::{c_char, c_int, c_long, c_void, size_t};
use crate::errno::{set_errno, EINVAL};

/// `strcoll(a, b)`：按当前 locale 比较字符串（POSIX）。
///
/// **本系统只有 C locale**（无 locale 数据库，`setlocale` 已判不支持）⇒ 在 C locale 下
/// `strcoll` 与 `strcmp` **逐字节等价**（POSIX 明确规定）。故直接委托 `strcmp`——
/// **不是近似，是 C locale 下的定义**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcoll(a: *const c_char, b: *const c_char) -> c_int {
    unsafe { crate::string::strcmp(a, b) }
}

/// `strxfrm(dest, src, n)`：按当前 locale 变换字符串（POSIX）。
///
/// C locale 下变换即**恒等** ⇒ 等价 `strncpy`，返回 `strlen(src)`（POSIX 允许返回所需长度）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strxfrm(dest: *mut c_char, src: *const c_char, n: size_t) -> size_t {
    let s = unsafe { crate::stdio::cstr_bytes(src) };
    if !dest.is_null() && n > 0 {
        let k = s.len().min(n - 1);
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), dest as *mut u8, k);
            *dest.add(k) = 0;
        }
    }
    s.len()
}

/// `fgetpos(stream, pos)`：取当前文件位置（POSIX）。
///
/// 本实现把 `fpos_t` 定义为 `long`（见 `<stdio.h>`）⇒ 委托 `ftell`，**真实语义**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetpos(stream: *mut c_void, pos: *mut c_long) -> c_int {
    if stream.is_null() || pos.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let r = unsafe { crate::stdio::ftell(stream as *mut crate::stdio::FILE) };
    if r < 0 {
        return -1;
    }
    unsafe { *pos = r as c_long };
    0
}

/// `fsetpos(stream, pos)`：设文件位置（POSIX）。委托 `fseek(SEEK_SET)`，**真实语义**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fsetpos(stream: *mut c_void, pos: *const c_long) -> c_int {
    if stream.is_null() || pos.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { crate::stdio::fseek(stream as *mut crate::stdio::FILE, *pos, 0) }
}

// `setbuf` / `setvbuf` **已移到 `libc/src/stdio.rs` 真实实现**（2026-10）。
//
// 原先这里是无缓冲时代的「刻意的空操作」，理由写的是「本 libc 的 stdio 不做用户态缓冲」。
// 写侧缓冲层落地后该前提不再成立，空操作于是从「诚实」退化成**能力谎言**
// （调用方请求有缓冲、拿到 0，实际每次仍直写）。真实实现必须能看到 `FILE` 的缓冲字段，
// 故放在 `stdio.rs`。
