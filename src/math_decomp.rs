//! 浮点分解/重组：frexp / ldexp（3P6-2 第二波）。
//!
//! **来路**：由 MPC 的真实编译报错驱动——
//!   radius.c:646:11: error: call to undeclared function 'frexp'
//!
//! 实现走 IEEE-754 位分解（不查表、不迭代），故对任意有限值都精确：
//! frexp 把 x 写成 m * 2^e 且 |m| ∈ [0.5, 1)；ldexp 是它的逆运算。

use crate::ctypes::c_int;

/// 2^1023（用于 ldexp 的分步缩放，避免中间溢出）。
const TWO_POW_1023: f64 = f64::from_bits(2046u64 << 52);
/// 2^-1022（最小正规数）。
const TWO_POW_M1022: f64 = f64::from_bits(1u64 << 52);

/// frexp(x, *exp)：分解为 m * 2^e，|m| ∈ [0.5, 1)，e 写入 *exp。
///
/// 边界（C 标准）：x == 0 ⇒ m = 0、e = 0；±Inf/NaN ⇒ 原样返回、e = 0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexp(x: f64, exp: *mut c_int) -> f64 {
    let bits = x.to_bits();
    let sign = bits & (1u64 << 63);
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & 0x000F_FFFF_FFFF_FFFF;
    unsafe {
        if !exp.is_null() {
            *exp = 0;
        }
    }
    if raw_exp == 0x7FF {
        return x; // Inf / NaN：原样返回，e = 0
    }
    if raw_exp == 0 && frac == 0 {
        return x; // ±0
    }
    let (e, m) = if raw_exp == 0 {
        // 次正规数：先乘 2^64 规格化（纯数学缩放，避免位运算边界）。
        let scaled = x * 18446744073709551616.0; // 2^64
        let sb = scaled.to_bits();
        let se = ((sb >> 52) & 0x7FF) as i32;
        (se - 1022 - 64, sign | (1022u64 << 52) | (sb & 0x000F_FFFF_FFFF_FFFF))
    } else {
        (raw_exp - 1022, sign | (1022u64 << 52) | frac)
    };
    unsafe {
        if !exp.is_null() {
            *exp = e;
        }
    }
    f64::from_bits(m)
}

/// ldexp(x, e)：返回 x * 2^e（frexp 的逆运算）。
///
/// 分步缩放，避免 e 很大/很小时中间结果溢出；下溢按 IEEE 规则自然发生（不报错）。
#[unsafe(no_mangle)]
pub extern "C" fn ldexp(x: f64, e: c_int) -> f64 {
    let mut r = x;
    let mut n = e;
    while n >= 1023 {
        r *= TWO_POW_1023;
        n -= 1023;
    }
    while n <= -1022 {
        r *= TWO_POW_M1022;
        n += 1022;
    }
    r * f64::from_bits(((n + 1023) as u64) << 52)
}