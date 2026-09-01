//! C ABI 基础类型与常量（S04：显式定义，不依赖 host 布局巧合）。
//!
//! 目标平台恒为 x86_64（本项目唯一构建目标），故按 LP64 数据模型显式定义：
//! - `c_long` = i64，`c_ulong` = u64（x86_64 Linux ABI）；
//! - `size_t` = usize = u64，`ssize_t` = isize = i64；
//! - `c_int` = i32，`c_char` = i8（x86_64 有符号 char）。
//!
//! 这些与 libsys 侧系统调用实参（u64）对齐，经 `as u64` / `as usize`
//! 转递。所有指针宽度断言由编译器保证（usize == u64）。

/// `size_t`：无符号大小类型。
pub type size_t = usize;
/// `ssize_t`：有符号大小类型。
pub type ssize_t = isize;
/// `c_int`：`int`。
pub type c_int = i32;
/// `c_uint`：`unsigned int`。
pub type c_uint = u32;
/// `c_long`：`long`（LP64 → 64 位）。
pub type c_long = i64;
/// `c_ulong`：`unsigned long`。
pub type c_ulong = u64;
/// `c_longlong`：`long long`。
pub type c_longlong = i64;
/// `c_ulonglong`：`unsigned long long`。
pub type c_ulonglong = u64;
/// `c_char`：`char`（x86_64 有符号）。
pub type c_char = i8;
/// `c_schar`：`signed char`。
pub type c_schar = i8;
/// `c_uchar`：`unsigned char`。
pub type c_uchar = u8;
/// `c_short`：`short`。
pub type c_short = i16;
/// `c_ushort`：`unsigned short`。
pub type c_ushort = u16;
/// `c_float`：`float`（f32）。
pub type c_float = f32;
/// `c_double`：`double`（f64）。
pub type c_double = f64;
/// `c_void`：`void`。
pub type c_void = core::ffi::c_void;

/// NULL 指针。
pub const NULL: *mut c_void = core::ptr::null_mut();

/// 无符号整数最大值（`UINT_MAX`）。
pub const UINT_MAX: c_uint = u32::MAX;
/// `INT_MAX`。
pub const INT_MAX: c_int = i32::MAX;
/// `INT_MIN`。
pub const INT_MIN: c_int = i32::MIN;
/// `LONG_MAX`。
pub const LONG_MAX: c_long = i64::MAX;
/// `SIZE_MAX`。
pub const SIZE_MAX: size_t = usize::MAX;
/// `EOF`：流结束 / 错误哨兵（-1）。
pub const EOF: c_int = -1;

// 宏 RAND_MAX：`rand()` 返回的最大值（本实现为 i32::MAX）。
pub const RAND_MAX: c_int = i32::MAX;
