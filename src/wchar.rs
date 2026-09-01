//! 宽字符（wchar_t）函数族（S04：显式定义，不依赖 host 布局）。
//!
//! 本目标 wchar_t = i32（x86_64 LP64）。当前无 locale/多字节编码支持，
//! 采用**逐字节扩展**编码：每个字节作为一个代码点（与 stdio `%lc/%ls\` 一致）。
//! 即 mbrtowc 把单个字节直接映射到 [0,255] 的宽字符；wcrtomb 反向。
//! 这一诚实简化在无 locale 的裸机上是确定性的（S09 宁缺毋假）。

use crate::ctypes::{c_char, size_t, wchar_t};

/// `wcslen(s)`：宽字符串长度（不含终止 NUL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(s: *const wchar_t) -> size_t {
    if s.is_null() {
        return 0;
    }
    let mut n: size_t = 0;
    unsafe {
        while *s.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// `wcscmp(a, b)`：宽字符串比较。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscmp(a: *const wchar_t, b: *const wchar_t) -> crate::ctypes::c_int {
    unsafe {
        let mut i = 0usize;
        loop {
            let ca = *a.add(i);
            let cb = *b.add(i);
            if ca != cb {
                return if ca < cb { -1 } else { 1 };
            }
            if ca == 0 {
                return 0;
            }
            i += 1;
        }
    }
}

/// `wcscpy(dst, src)`：宽字符串拷贝。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscpy(dst: *mut wchar_t, src: *const wchar_t) -> *mut wchar_t {
    unsafe {
        let mut i = 0usize;
        loop {
            let c = *src.add(i);
            *dst.add(i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
        dst
    }
}

/// `wcsncpy(dst, src, n)`：宽字符串受限拷贝（不足补 NUL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsncpy(dst: *mut wchar_t, src: *const wchar_t, n: size_t) -> *mut wchar_t {
    unsafe {
        let mut i = 0usize;
        while i < n {
            let c = if i < wcslen(src) { *src.add(i) } else { 0 };
            *dst.add(i) = c;
            i += 1;
        }
        dst
    }
}

/// `wcscat(dst, src)`：宽字符串拼接。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscat(dst: *mut wchar_t, src: *const wchar_t) -> *mut wchar_t {
    unsafe {
        let len = wcslen(dst);
        let mut i = 0usize;
        loop {
            let c = *src.add(i);
            *dst.add(len + i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
        dst
    }
}

/// `wcschr(s, c)`：宽字符串中找字符。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcschr(s: *const wchar_t, c: wchar_t) -> *mut wchar_t {
    unsafe {
        let mut i = 0usize;
        loop {
            let sc = *s.add(i);
            if sc == c {
                return s.add(i) as *mut wchar_t;
            }
            if sc == 0 {
                return core::ptr::null_mut();
            }
            i += 1;
        }
    }
}

/// `mbrtowc(pwc, s, n, ps)`：把多字节序列转换为宽字符。
/// 本实现无多字节编码：s 首字节即宽字符（[0,255]），返回 1 或 0/（size_t)-1。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrtowc(
    pwc: *mut wchar_t,
    s: *const c_char,
    n: size_t,
    _ps: *mut crate::ctypes::c_void,
) -> size_t {
    unsafe {
        if s.is_null() {
            return 0;
        }
        if n == 0 {
            return 0;
        }
        let c = *s as u8 as wchar_t;
        if c == 0 {
            if !pwc.is_null() {
                *pwc = 0;
            }
            return 0;
        }
        if !pwc.is_null() {
            *pwc = c;
        }
        1
    }
}

/// `wcrtomb(s, wc, ps)`：把宽字符转换为多字节序列。
/// 无多字节编码：仅 [0,255] 可转（1 字节）；其余返回 (size_t)-1 并置 EILSEQ。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb(
    s: *mut c_char,
    wc: wchar_t,
    _ps: *mut crate::ctypes::c_void,
) -> size_t {
    unsafe {
        if s.is_null() {
            return 0;
        }
        if wc < 0 || wc > 0xFF {
            crate::errno::set_errno(crate::errno::EILSEQ);
            return usize::MAX;
        }
        *s = wc as u8 as c_char;
        1
    }
}

/// `mbsrtowcs(dst, src, len, ps)`：把多字节字符串转换为宽字符串。
/// 返回写入的宽字符数（不含 NUL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsrtowcs(
    dst: *mut wchar_t,
    src: *mut *const c_char,
    len: size_t,
    _ps: *mut crate::ctypes::c_void,
) -> size_t {
    unsafe {
        if src.is_null() || (*src).is_null() {
            return usize::MAX;
        }
        let mut p = *src;
        let mut i = 0usize;
        loop {
            let b = *p as u8 as wchar_t;
            if b == 0 {
                if !dst.is_null() {
                    *dst.add(i) = 0;
                }
                *src = core::ptr::null();
                return i;
            }
            if i >= len {
                *src = p; // 未转换完，更新 src。
                return i;
            }
            if !dst.is_null() {
                *dst.add(i) = b;
            }
            i += 1;
            p = p.add(1);
        }
    }
}

/// `wcsrtombs(dst, src, len, ps)`：把宽字符串转换为多字节字符串。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsrtombs(
    dst: *mut c_char,
    src: *mut *const wchar_t,
    len: size_t,
    _ps: *mut crate::ctypes::c_void,
) -> size_t {
    unsafe {
        if src.is_null() || (*src).is_null() {
            return usize::MAX;
        }
        let mut p = *src;
        let mut i = 0usize;
        loop {
            let w = *p;
            if w == 0 {
                if !dst.is_null() {
                    *dst.add(i) = 0;
                }
                *src = core::ptr::null();
                return i;
            }
            if w < 0 || w > 0xFF || i >= len {
                *src = p;
                return usize::MAX; // 不可转（含空间不足）。
            }
            if !dst.is_null() {
                *dst.add(i) = w as u8 as c_char;
            }
            i += 1;
            p = p.add(1);
        }
    }
}

/// `wcstombs(dst, src, n)`：多字节（单字节）→ 宽字符便捷封装。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs(dst: *mut wchar_t, src: *const c_char, n: size_t) -> size_t {
    unsafe {
        if src.is_null() {
            return usize::MAX;
        }
        let mut srcp: *const c_char = src;
        mbsrtowcs(dst, &mut srcp, n, core::ptr::null_mut())
    }
}

/// `wcstombs(dst, src, n)`：宽字符 → 多字节（单字节）便捷封装。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs(dst: *mut c_char, src: *const wchar_t, n: size_t) -> size_t {
    unsafe {
        if src.is_null() {
            return usize::MAX;
        }
        let mut srcp: *const wchar_t = src;
        wcsrtombs(dst, &mut srcp, n, core::ptr::null_mut())
    }
}
