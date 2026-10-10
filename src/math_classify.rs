//! C99 浮点分类（`<math.h>` 7.12.3）：`fpclassify` / `isnan` / `isinf` / `isfinite` /
//! `isnormal` / `signbit` 的 **C ABI 函数**形态。
//!
//! ## 为什么需要（3P6-3 的真实阻塞，不是"看起来该有"）
//!
//! 本 libc 的 `<math.h>` 此前**只有 `FP_*` 常量、没有这些函数也没有宏**。后果有两层：
//!
//! 1. **C 程序**用不了 `isnan(x)` 这类 C99 分类（本该是宏）；
//! 2. **C++ 的 `std::isnan` 等更用不了**——libstdc++ 的 `<cmath>` 是 `#undef` 掉宏后
//!    用 `using ::isnan;` 引入**函数**的，故 `::isnan` 必须真实存在。这也是
//!    `_GLIBCXX_USE_C99_MATH` 能打开的前提之一。
//!
//! ## 实现取舍
//!
//! 走 Rust `core` 的 `f64` 内建谓词（`is_nan`/`is_infinite`/`is_normal`/
//! `is_sign_negative`），**不自己拆 IEEE-754 位**——那是编译器/标准库已经保证正确的
//! 语义，手写位运算只会引入与 NaN 载荷、负零、次正规数相关的边界错误。
//!
//! **诚实边界**：只提供 `double` 版（C99 的宏形态才是"接受任意实参类型"的入口，
//! 见 `<math.h>`；`float`/`long double` 的 C++ 重载由 libstdc++ 自己给）。

use crate::ctypes::c_int;


/// `NAN` 的真值（IEEE-754 quiet NaN 位型）。
///
/// **为什么是 libc 的常量对象而不是编译器内建**：`<math.h>` 原来写
/// `#define NAN (__builtin_nanf(""))`，而**机内 tcc 不提供这些内建**——实测任何用
/// `NAN`/`INFINITY` 的 C 程序在机内 tcc 下链接失败：
/// `tcc: error: unresolved reference to '__builtin_nanf'`。
/// libc 用位模式提供真值，不依赖任何编译器内建。
#[unsafe(no_mangle)]
pub static __boruix_nan: f64 = f64::from_bits(0x7FF8_0000_0000_0000);

/// `INFINITY` / `HUGE_VAL` 的真值（IEEE-754 正无穷位型）。理由同 [`__boruix_nan`]。
#[unsafe(no_mangle)]
pub static __boruix_inf: f64 = f64::from_bits(0x7FF0_0000_0000_0000);

/// `<math.h>` 的 `FP_*` 取值（与头文件**必须一致**）。
const FP_NAN: c_int = 0;
const FP_INFINITE: c_int = 1;
const FP_ZERO: c_int = 2;
const FP_SUBNORMAL: c_int = 3;
const FP_NORMAL: c_int = 4;

/// `fpclassify(x)`：返回 `FP_*` 分类码。
#[unsafe(no_mangle)]
pub extern "C" fn fpclassify(x: f64) -> c_int {
    if x.is_nan() {
        FP_NAN
    } else if x.is_infinite() {
        FP_INFINITE
    } else if x == 0.0 {
        FP_ZERO
    } else if x.is_normal() {
        FP_NORMAL
    } else {
        FP_SUBNORMAL
    }
}

/// `isnan(x)`：NaN 返回非 0。
#[unsafe(no_mangle)]
pub extern "C" fn isnan(x: f64) -> c_int {
    if x.is_nan() { 1 } else { 0 }
}

/// `isinf(x)`：正无穷返回 1、负无穷返回 -1、其余 0（POSIX 语义）。
#[unsafe(no_mangle)]
pub extern "C" fn isinf(x: f64) -> c_int {
    if x.is_infinite() {
        if x > 0.0 { 1 } else { -1 }
    } else {
        0
    }
}

/// `isfinite(x)`：既非 NaN 也非无穷返回非 0。
#[unsafe(no_mangle)]
pub extern "C" fn isfinite(x: f64) -> c_int {
    if x.is_finite() { 1 } else { 0 }
}

/// `isnormal(x)`：规格化数（非 0、非次正规、非 NaN/无穷）返回非 0。
#[unsafe(no_mangle)]
pub extern "C" fn isnormal(x: f64) -> c_int {
    if x.is_normal() { 1 } else { 0 }
}

/// `signbit(x)`：符号位为 1（含 `-0.0`）返回非 0。
#[unsafe(no_mangle)]
pub extern "C" fn signbit(x: f64) -> c_int {
    if x.is_sign_negative() { 1 } else { 0 }
}