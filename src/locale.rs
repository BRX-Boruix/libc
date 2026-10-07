//! locale（**诚实最小面**）——见 <locale.h> 的说明。
//!
//! 3P6-2 第二波：由 MPC 的真实编译报错驱动（get_x.c: fatal error: 'locale.h' file not found）。
//!
//! **诚实边界（S09）**：本系统没有 locale 数据库。setlocale 只认 "C"/"POSIX" 并返回其名，
//! 其余如实返回 NULL（不假装切换成功）；localeconv 返回 C locale 的固定值。

use crate::ctypes::c_char;

/// 当前 locale 名（恒为 C.UTF-8；静态存储，POSIX 允许返回静态指针）。
static C_LOCALE: [c_char; 8] = [
    b'C' as c_char, b'.' as c_char, b'U' as c_char, b'T' as c_char, b'F' as c_char, b'-' as c_char,
    b'8' as c_char, 0,
];

/// struct lconv（只含有数据来源的字段；与 <locale.h> 逐字段一致）。
#[repr(C)]
pub struct lconv {
    pub decimal_point: *mut c_char,
    pub thousands_sep: *mut c_char,
    pub grouping: *mut c_char,
}

/// setlocale(category, locale)：见模块说明。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlocale(_category: crate::ctypes::c_int, locale: *const c_char) -> *mut c_char {
    unsafe {
        // locale == NULL 是「查询当前 locale」（POSIX）⇒ 返回当前值。
        if locale.is_null() {
            return core::ptr::addr_of!(C_LOCALE) as *mut c_char;
        }
        let p = locale as *const u8;
        let is_c = (*p == b'C' && *p.add(1) == 0)
            || (*p == b'P'
                && *p.add(1) == b'O'
                && *p.add(2) == b'S'
                && *p.add(3) == b'I'
                && *p.add(4) == b'X'
                && *p.add(5) == 0);
        if is_c {
            core::ptr::addr_of!(C_LOCALE) as *mut c_char
        } else {
            core::ptr::null_mut() // 无该 locale 的数据 ⇒ 如实 NULL
        }
    }
}

/// localeconv()：C locale 的固定值（小数点 "."，无千位分隔与分组）。
#[unsafe(no_mangle)]
pub extern "C" fn localeconv() -> *mut lconv {
    static mut CONV: lconv = lconv {
        decimal_point: core::ptr::null_mut(),
        thousands_sep: core::ptr::null_mut(),
        grouping: core::ptr::null_mut(),
    };
    static mut DOT: [c_char; 2] = [b'.' as c_char, 0];
    static mut EMPTY: [c_char; 1] = [0];
    unsafe {
        CONV.decimal_point = core::ptr::addr_of_mut!(DOT) as *mut c_char;
        CONV.thousands_sep = core::ptr::addr_of_mut!(EMPTY) as *mut c_char;
        CONV.grouping = core::ptr::addr_of_mut!(EMPTY) as *mut c_char;
        core::ptr::addr_of_mut!(CONV)
    }
}