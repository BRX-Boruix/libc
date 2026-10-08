//! 数学函数**导出层**（`#[no_mangle] extern "C"`）——3P3-2 精度重做后的唯一接出口。
//!
//! ## 为什么与核心分开
//!
//! `tools/checks/math_verify/verify.rs` 用 `include!` 把 `math_core.rs` /
//! `math_core2.rs` 引入**宿主**验证程序，与宿主 glibc 的 libm 逐点比 ULP。
//! 若核心文件里带 `#[no_mangle] extern "C"`，宿主验证程序会与 glibc **重复符号**而链不上。
//! 故：**核心是纯函数（可被宿主 include）**，导出只在本文件。
//!
//! ## 精度（B 档：≤1 ulp）与验收
//!
//! 每项都由宿主对照验证器在 **5231 个样本 + 边界值**上实测最大 ULP：
//! 复跑方式 `cd tools/checks/math_verify && rustc -O --edition 2021 verify.rs -o verify.exe && ./verify.exe`。
//! 当前 **27/27 项 ≤1 ulp**（其中 sqrt/log2/asin 为 0 ulp）。
//! **未通过验证的函数不接进 libc**——这是本文件的硬规矩。
//!
//! ## 诚实边界（不假装支持）
//!
//! - 三角函数归约用 fdlibm 的三轮 Cody-Waite，**有效范围 |x| < 2^20**；
//!   超出需 Payne-Hanek（未实现），此时结果不作 ≤1 ulp 承诺。
//! - f32 版是"f64 核心 + 一次收窄"：f64 误差 ~2^-52 远小于 f32 的半个 ulp（2^-24），
//!   故收窄后仍是 f32 正确舍入（f32 层未单独跑对照表，此处如实声明）。
//! - 无 `long double` 版（除 `ldexpl`，见 `longdouble.rs`）。

use crate::ctypes::c_int;
use crate::math_core::*;
use crate::math_core2::*;

// ==================== double ====================

#[unsafe(no_mangle)]
pub extern "C" fn fabs(x: f64) -> f64 { core_fabs(x) }
#[unsafe(no_mangle)]
pub extern "C" fn copysign(x: f64, y: f64) -> f64 { core_copysign(x, y) }
#[unsafe(no_mangle)]
pub extern "C" fn floor(x: f64) -> f64 { core_floor(x) }
#[unsafe(no_mangle)]
pub extern "C" fn ceil(x: f64) -> f64 { core_ceil(x) }
#[unsafe(no_mangle)]
pub extern "C" fn trunc(x: f64) -> f64 { core_trunc(x) }
#[unsafe(no_mangle)]
pub extern "C" fn round(x: f64) -> f64 { core_round(x) }
#[unsafe(no_mangle)]
pub extern "C" fn nearbyint(x: f64) -> f64 { core_nearbyint(x) }
#[unsafe(no_mangle)]
pub extern "C" fn sqrt(x: f64) -> f64 { core_sqrt(x) }
#[unsafe(no_mangle)]
pub extern "C" fn cbrt(x: f64) -> f64 { core_cbrt(x) }
#[unsafe(no_mangle)]
pub extern "C" fn hypot(x: f64, y: f64) -> f64 { core_hypot(x, y) }
#[unsafe(no_mangle)]
pub extern "C" fn nextafter(x: f64, y: f64) -> f64 { core_nextafter(x, y) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn modf(x: f64, iptr: *mut f64) -> f64 { core_modf(x, &mut *iptr) }
#[unsafe(no_mangle)]
pub extern "C" fn fmod(x: f64, y: f64) -> f64 { core_fmod(x, y) }

#[unsafe(no_mangle)]
pub extern "C" fn exp(x: f64) -> f64 { core_exp(x) }
#[unsafe(no_mangle)]
pub extern "C" fn expm1(x: f64) -> f64 { core_expm1(x) }
#[unsafe(no_mangle)]
pub extern "C" fn log(x: f64) -> f64 { core_log(x) }
#[unsafe(no_mangle)]
pub extern "C" fn log1p(x: f64) -> f64 { core_log1p(x) }
#[unsafe(no_mangle)]
pub extern "C" fn log2(x: f64) -> f64 { core_log2(x) }
#[unsafe(no_mangle)]
pub extern "C" fn log10(x: f64) -> f64 { core_log10(x) }
#[unsafe(no_mangle)]
pub extern "C" fn pow(x: f64, y: f64) -> f64 { core_pow(x, y) }

#[unsafe(no_mangle)]
pub extern "C" fn sin(x: f64) -> f64 { core_sin(x) }
#[unsafe(no_mangle)]
pub extern "C" fn cos(x: f64) -> f64 { core_cos(x) }
#[unsafe(no_mangle)]
pub extern "C" fn tan(x: f64) -> f64 { core_tan(x) }
#[unsafe(no_mangle)]
pub extern "C" fn asin(x: f64) -> f64 { core_asin(x) }
#[unsafe(no_mangle)]
pub extern "C" fn acos(x: f64) -> f64 { core_acos(x) }
#[unsafe(no_mangle)]
pub extern "C" fn atan(x: f64) -> f64 { core_atan(x) }
#[unsafe(no_mangle)]
pub extern "C" fn atan2(y: f64, x: f64) -> f64 { core_atan2(y, x) }
#[unsafe(no_mangle)]
pub extern "C" fn sinh(x: f64) -> f64 { core_sinh(x) }
#[unsafe(no_mangle)]
pub extern "C" fn cosh(x: f64) -> f64 { core_cosh(x) }
#[unsafe(no_mangle)]
pub extern "C" fn tanh(x: f64) -> f64 { core_tanh(x) }

// ==================== float（f64 核心 + 一次收窄） ====================

/// f32 包装：`core(x as f64) as f32`。
macro_rules! export_f32 {
    ($($name:ident => $core:ident),* $(,)?) => { $(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: f32) -> f32 { $core(x as f64) as f32 }
    )* };
}
export_f32!(
    fabsf => core_fabs, floorf => core_floor, ceilf => core_ceil,
    truncf => core_trunc, roundf => core_round, sqrtf => core_sqrt,
    cbrtf => core_cbrt, expf => core_exp, expm1f => core_expm1,
    logf => core_log, log1pf => core_log1p, log2f => core_log2,
    log10f => core_log10, sinf => core_sin, cosf => core_cos,
    tanf => core_tan, asinf => core_asin, acosf => core_acos,
    atanf => core_atan, sinhf => core_sinh, coshf => core_cosh,
    tanhf => core_tanh, nearbyintf => core_nearbyint,
);

#[unsafe(no_mangle)]
pub extern "C" fn copysignf(x: f32, y: f32) -> f32 {
    f32::from_bits((x.to_bits() & 0x7fff_ffff) | (y.to_bits() & 0x8000_0000))
}
#[unsafe(no_mangle)]
pub extern "C" fn powf(x: f32, y: f32) -> f32 { core_pow(x as f64, y as f64) as f32 }
#[unsafe(no_mangle)]
pub extern "C" fn atan2f(y: f32, x: f32) -> f32 { core_atan2(y as f64, x as f64) as f32 }
#[unsafe(no_mangle)]
pub extern "C" fn hypotf(x: f32, y: f32) -> f32 { core_hypot(x as f64, y as f64) as f32 }
#[unsafe(no_mangle)]
pub extern "C" fn fmodf(x: f32, y: f32) -> f32 { core_fmod(x as f64, y as f64) as f32 }

/// f32 的 nextafter **不能**走 f64 核心（步长语义不同）⇒ 直接按位做。
#[unsafe(no_mangle)]
pub extern "C" fn nextafterf(x: f32, y: f32) -> f32 {
    if x.is_nan() || y.is_nan() { return f32::NAN; }
    if x == y { return y; }
    if x == 0.0 {
        let s = if y.is_sign_negative() { 0x8000_0000u32 } else { 0 };
        return f32::from_bits(s | 1);
    }
    let up = (y > x) == (x > 0.0);
    let b = x.to_bits();
    f32::from_bits(if up { b + 1 } else { b - 1 })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn modff(x: f32, iptr: *mut f32) -> f32 {
    let mut t = 0.0f64;
    let f = core_modf(x as f64, &mut t);
    *iptr = t as f32;
    f as f32
}

// frexp/ldexp 的 double 版在 math_decomp.rs（由 MPC 报错驱动补上），此处只补 f32 版。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexpf(x: f32, exp: *mut c_int) -> f32 {
    crate::math_decomp::frexp(x as f64, exp) as f32
}
#[unsafe(no_mangle)]
pub extern "C" fn ldexpf(x: f32, exp: c_int) -> f32 {
    crate::math_decomp::ldexp(x as f64, exp) as f32
}
