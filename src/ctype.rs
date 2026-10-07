//! 字符分类与转换函数（C ABI）。
//!
//! 输入约定（C11）：\`isxxx\` 系列参数须为 \`unsigned char\` 的值或 EOF；负数
//! （非 EOF）是未定义行为。本实现把参数截断为 u8 处理，并**不**对 EOF 特判
//! （调用方不应传入 EOF 到这些函数，C 契约）。全部纯逻辑、可移植。

use crate::ctypes::c_int;

/// 参数按 unsigned char 处理。
#[inline]
fn u(c: c_int) -> u8 {
    (c & 0xFF) as u8
}

/// \`isalpha(c)\`：字母。
#[unsafe(no_mangle)]
pub extern "C" fn isalpha(c: c_int) -> c_int {
    let c = u(c);
    ((c >= b'a' && c <= b'z') || (c >= b'A' && c <= b'Z')) as c_int
}

/// isascii(c)：c 是否为 7 位 ASCII（0..=127）。
///
/// **来路（3P6-2 第二波，真实报错驱动，不预猜）**：交叉构建 GMP 时报
///   printf/doprnt.c:592:17: error: call to undeclared function 'isascii'
/// 本 libc 此前**既没实现也没声明**它（属「整项缺失」，与 getc/putc/getuid 同类）。
/// POSIX.1-2008 已把它移出标准，但现实代码大量使用；语义就是「高位全 0」。
#[unsafe(no_mangle)]
pub extern "C" fn isascii(c: c_int) -> c_int {
    ((c & !0x7F) == 0) as c_int
}

/// \`isdigit(c)\`：十进制数字。
#[unsafe(no_mangle)]
pub extern "C" fn isdigit(c: c_int) -> c_int {
    let c = u(c);
    (c >= b'0' && c <= b'9') as c_int
}

/// \`isalnum(c)\`：字母或数字。
#[unsafe(no_mangle)]
pub extern "C" fn isalnum(c: c_int) -> c_int {
    let c = u(c);
    ((c >= b'a' && c <= b'z') || (c >= b'A' && c <= b'Z') || (c >= b'0' && c <= b'9')) as c_int
}

/// \`isupper(c)\`：大写字母。
#[unsafe(no_mangle)]
pub extern "C" fn isupper(c: c_int) -> c_int {
    let c = u(c);
    (c >= b'A' && c <= b'Z') as c_int
}

/// \`islower(c)\`：小写字母。
#[unsafe(no_mangle)]
pub extern "C" fn islower(c: c_int) -> c_int {
    let c = u(c);
    (c >= b'a' && c <= b'z') as c_int
}

/// \`isspace(c)\`：空白（空格、\\t、\\n、\\v、\\f、\\r）。
#[unsafe(no_mangle)]
pub extern "C" fn isspace(c: c_int) -> c_int {
    let c = u(c);
    (c == b' ' || (c >= b'\t' && c <= b'\r')) as c_int
}

/// \`isxdigit(c)\`：十六进制数字。
#[unsafe(no_mangle)]
pub extern "C" fn isxdigit(c: c_int) -> c_int {
    let c = u(c);
    ((c >= b'0' && c <= b'9') || (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F')) as c_int
}

/// \`isprint(c)\`：可打印字符（0x20..0x7E）。
#[unsafe(no_mangle)]
pub extern "C" fn isprint(c: c_int) -> c_int {
    let c = u(c);
    (c >= 0x20 && c <= 0x7E) as c_int
}

/// \`ispunct(c)\`：标点。
#[unsafe(no_mangle)]
pub extern "C" fn ispunct(c: c_int) -> c_int {
    let c = u(c);
    ((c >= 0x21 && c <= 0x2F) || (c >= 0x3A && c <= 0x40) || (c >= 0x5B && c <= 0x60) || (c >= 0x7B && c <= 0x7E)) as c_int
}

/// \`iscntrl(c)\`：控制字符。
#[unsafe(no_mangle)]
pub extern "C" fn iscntrl(c: c_int) -> c_int {
    let c = u(c);
    ((c < 0x20) || c == 0x7F) as c_int
}

/// \`isgraph(c)\`：可打印且非空白。
#[unsafe(no_mangle)]
pub extern "C" fn isgraph(c: c_int) -> c_int {
    let c = u(c);
    (c >= 0x21 && c <= 0x7E) as c_int
}

/// \`isblank(c)\`：空白（空格或 \\t）。
#[unsafe(no_mangle)]
pub extern "C" fn isblank(c: c_int) -> c_int {
    let c = u(c);
    (c == b' ' || c == b'\t') as c_int
}

/// \`tolower(c)\`：转小写（非字母原样）。
#[unsafe(no_mangle)]
pub extern "C" fn tolower(c: c_int) -> c_int {
    let c = u(c);
    if c >= b'A' && c <= b'Z' {
        (c + 32) as c_int
    } else {
        c as c_int
    }
}

/// \`toupper(c)\`：转大写（非字母原样）。
#[unsafe(no_mangle)]
pub extern "C" fn toupper(c: c_int) -> c_int {
    let c = u(c);
    if c >= b'a' && c <= b'z' {
        (c - 32) as c_int
    } else {
        c as c_int
    }
}
