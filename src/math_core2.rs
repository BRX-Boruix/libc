// 超越函数（第二部分）。**单独成文件是为了绕开「往 math_core.rs 追加」这个脆弱操作**——
// 另写新文件一次成型，比字符串拼接可靠得多。
//
// 与 math_core.rs 同属一套：纯计算、无 syscall，故宿主验证器可 include! 两份一起对照 libm。
// 依赖 base 部分的 core_fabs/core_copysign/core_sqrt/core_trunc。

use super::math_core::*;

const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INV_LN2: f64 = 1.44269504088896338700e+00;
const PIO2_HI: f64 = 1.57079632673412561417e+00;
const PIO2_MID: f64 = 6.07710050650619224932e-11;
const PIO2_LO: f64 = 2.02226624879595063154e-21;
const TWO_OVER_PI: f64 = 6.36619772367581382433e-01;
const PI: f64 = 3.141592653589793;
const PIO2: f64 = 1.5707963267948966;

/// 乘 2^k（分步，避免中间溢出/下溢）。
fn scale2(mut v: f64, mut k: i64) -> f64 {
    while k > 1000 { v *= f64::from_bits(0x7fe0_0000_0000_0000); k -= 1000; }
    while k < -1000 { v *= f64::from_bits(0x0010_0000_0000_0000); k += 1000; }
    v * f64::from_bits(((1023 + k) as u64) << 52)
}

/// `exp`：k=round(x/ln2)，r=x-k·ln2（Cody-Waite 双字），exp(r) 用 Taylor 至 r¹⁴，再乘 2^k。
pub fn core_exp(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x > 709.782712893384 { return f64::INFINITY; }
    if x < -745.1332191019411 { return 0.0; }
    let k = (x * INV_LN2 + if x >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let kf = k as f64;
    let r = (x - kf * LN2_HI) - kf * LN2_LO;
    let mut s = 1.0 / 87178291200.0;
    s = s * r + 1.0 / 6227020800.0;
    s = s * r + 1.0 / 479001600.0;
    s = s * r + 1.0 / 39916800.0;
    s = s * r + 1.0 / 3628800.0;
    s = s * r + 1.0 / 362880.0;
    s = s * r + 1.0 / 40320.0;
    s = s * r + 1.0 / 5040.0;
    s = s * r + 1.0 / 720.0;
    s = s * r + 1.0 / 120.0;
    s = s * r + 1.0 / 24.0;
    s = s * r + 1.0 / 6.0;
    s = s * r + 0.5;
    s = s * r + 1.0;
    s = s * r + 1.0;
    scale2(s, k)
}

/// `expm1`：小 |x| 用级数（避免 exp(x)-1 的抵消）。
pub fn core_expm1(x: f64) -> f64 {
    if core_fabs(x) < 1e-5 {
        let x2 = x * x;
        return x + x2 * (0.5 + x * (1.0 / 6.0 + x * (1.0 / 24.0 + x / 120.0)));
    }
    core_exp(x) - 1.0
}

/// `log`：x=m·2^k，m∈[√2/2,√2)；s=(m-1)/(m+1)，log m = 2·Σ s^(2i+1)/(2i+1)。
pub fn core_log(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    let mut bits = x.to_bits();
    let mut k = (((bits >> 52) & 0x7ff) as i64) - 1023;
    if k == -1023 {
        let y = x * f64::from_bits(0x4350_0000_0000_0000);
        bits = y.to_bits();
        k = (((bits >> 52) & 0x7ff) as i64) - 1023 - 54;
    }
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > 1.4142135623730951 {
        m *= 0.5;
        k += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut p = 1.0 / 19.0;
    p = p * s2 + 1.0 / 17.0;
    p = p * s2 + 1.0 / 15.0;
    p = p * s2 + 1.0 / 13.0;
    p = p * s2 + 1.0 / 11.0;
    p = p * s2 + 1.0 / 9.0;
    p = p * s2 + 1.0 / 7.0;
    p = p * s2 + 1.0 / 5.0;
    p = p * s2 + 1.0 / 3.0;
    p = p * s2 + 1.0;
    let kf = k as f64;
    2.0 * s * p + (kf * LN2_HI + kf * LN2_LO)
}

pub fn core_log1p(x: f64) -> f64 {
    if core_fabs(x) < 1e-4 {
        let x2 = x * x;
        return x - x2 * (0.5 - x * (1.0 / 3.0 - x * (0.25 - x / 5.0)));
    }
    core_log(1.0 + x)
}

pub fn core_log2(x: f64) -> f64 { core_log(x) * INV_LN2 }
pub fn core_log10(x: f64) -> f64 { core_log(x) / 2.30258509299404568402e+00 }

/// `pow`：整数指数走反复平方（精确），其余 `exp(y·log x)`。**ULP 由验证器实测，不声称正确舍入。**
pub fn core_pow(x: f64, y: f64) -> f64 {
    if y == 0.0 { return 1.0; }
    if x == 1.0 { return 1.0; }
    if x.is_nan() || y.is_nan() { return f64::NAN; }
    if y == 1.0 { return x; }
    if y == 2.0 { return x * x; }
    if y == 0.5 && x >= 0.0 { return core_sqrt(x); }
    if y == core_trunc(y) && core_fabs(y) <= 1024.0 {
        let neg = y < 0.0;
        let mut n = core_fabs(y) as u64;
        let mut base = x;
        let mut acc = 1.0f64;
        while n > 0 {
            if n & 1 == 1 { acc *= base; }
            base *= base;
            n >>= 1;
        }
        return if neg { 1.0 / acc } else { acc };
    }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return if y > 0.0 { 0.0 } else { f64::INFINITY }; }
    core_exp(y * core_log(x))
}

fn reduce_pio2(x: f64) -> (i64, f64) {
    let n = (x * TWO_OVER_PI + if x >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let nf = n as f64;
    let r = ((x - nf * PIO2_HI) - nf * PIO2_MID) - nf * PIO2_LO;
    (n & 3, r)
}

fn sin_poly(r: f64) -> f64 {
    let r2 = r * r;
    let mut p = -1.0 / 6227020800.0;
    p = p * r2 + 1.0 / 39916800.0;
    p = p * r2 - 1.0 / 362880.0;
    p = p * r2 + 1.0 / 5040.0;
    p = p * r2 - 1.0 / 120.0;
    p = p * r2 + 1.0 / 6.0;
    r + r * r2 * p
}

fn cos_poly(r: f64) -> f64 {
    let r2 = r * r;
    1.0 - r2 * (0.5 - r2 * (1.0 / 24.0 - r2 * (1.0 / 720.0 - r2 * (1.0 / 40320.0
        - r2 * (1.0 / 3628800.0 - r2 * (1.0 / 479001600.0 - r2 / 87178291200.0))))))
}

pub fn core_sin(x: f64) -> f64 {
    if !x.is_finite() { return f64::NAN; }
    let (n, r) = reduce_pio2(x);
    match n { 0 => sin_poly(r), 1 => cos_poly(r), 2 => -sin_poly(r), _ => -cos_poly(r) }
}

pub fn core_cos(x: f64) -> f64 {
    if !x.is_finite() { return f64::NAN; }
    let (n, r) = reduce_pio2(x);
    match n { 0 => cos_poly(r), 1 => -sin_poly(r), 2 => -cos_poly(r), _ => sin_poly(r) }
}

pub fn core_tan(x: f64) -> f64 {
    if !x.is_finite() { return f64::NAN; }
    let (n, r) = reduce_pio2(x);
    let s = sin_poly(r);
    let c = cos_poly(r);
    if n & 1 == 0 { s / c } else { -c / s }
}

fn atan_poly(x: f64) -> f64 {
    let x2 = x * x;
    let mut s = 0.0;
    let mut k = 14i32;
    while k >= 0 {
        let c = 1.0 / (2.0 * k as f64 + 1.0);
        s = s * x2 + if k % 2 == 0 { c } else { -c };
        k -= 1;
    }
    x * s
}

pub fn core_atan(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax.is_infinite() { return core_copysign(PIO2, x); }
    if ax <= 1.0 {
        core_copysign(atan_poly(ax), x)
    } else {
        core_copysign(PIO2 - atan_poly(1.0 / ax), x)
    }
}

pub fn core_atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() { return f64::NAN; }
    if x > 0.0 { return core_atan(y / x); }
    if x < 0.0 {
        let a = core_atan(y / x);
        return if y >= 0.0 { a + PI } else { a - PI };
    }
    if y > 0.0 { return PIO2; }
    if y < 0.0 { return -PIO2; }
    match (y.is_sign_negative(), x.is_sign_negative()) {
        (false, false) => 0.0,
        (false, true) => PI,
        (true, false) => -0.0,
        (true, true) => -PI,
    }
}

pub fn core_asin(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax > 1.0 { return f64::NAN; }
    if ax == 1.0 { return core_copysign(PIO2, x); }
    core_atan2(x, core_sqrt((1.0 - ax) * (1.0 + ax)))
}

pub fn core_acos(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax > 1.0 { return f64::NAN; }
    if x == 1.0 { return 0.0; }
    if x == -1.0 { return PI; }
    core_atan2(core_sqrt((1.0 - ax) * (1.0 + ax)), x)
}

pub fn core_sinh(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax < 1e-5 { return x + x * x * x / 6.0; }
    if ax > 710.0 { return core_copysign(f64::INFINITY, x); }
    let e = core_exp(ax);
    core_copysign(0.5 * (e - 1.0 / e), x)
}

pub fn core_cosh(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax > 710.0 { return f64::INFINITY; }
    let e = core_exp(ax);
    0.5 * (e + 1.0 / e)
}

pub fn core_tanh(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax > 20.0 { return core_copysign(1.0, x); }
    let e = core_exp(2.0 * ax);
    core_copysign((e - 1.0) / (e + 1.0), x)
}

pub fn core_cbrt(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() { return x; }
    let ax = core_fabs(x);
    let mut y = f64::from_bits((ax.to_bits() / 3) + 0x2a51_45d2_0000_0000);
    for _ in 0..6 { y = y - (y - ax / (y * y)) / 3.0; }
    core_copysign(y, x)
}

pub fn core_hypot(x: f64, y: f64) -> f64 {
    let ax = core_fabs(x);
    let ay = core_fabs(y);
    if ax.is_infinite() || ay.is_infinite() { return f64::INFINITY; }
    if ax.is_nan() || ay.is_nan() { return f64::NAN; }
    let (hi, lo) = if ax > ay { (ax, ay) } else { (ay, ax) };
    if hi == 0.0 { return 0.0; }
    let r = lo / hi;
    hi * core_sqrt(1.0 + r * r)
}

pub fn core_nearbyint(x: f64) -> f64 {
    let t = core_trunc(x);
    let d = x - t;
    let ad = core_fabs(d);
    if ad < 0.5 { t } else if ad > 0.5 { t + core_copysign(1.0, x) } else {
        if (t as i64) % 2 == 0 { t } else { t + core_copysign(1.0, x) }
    }
}

pub fn core_nextafter(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() { return f64::NAN; }
    if x == y { return y; }
    if x == 0.0 { return core_copysign(f64::from_bits(1), y); }
    let up = (y > x) == (x > 0.0);
    let b = x.to_bits();
    f64::from_bits(if up { b + 1 } else { b - 1 })
}

// ---- float 版：在 f64 中算再舍入（f64 有 53 位有效位 ≫ f32 的 24 位）----
macro_rules! f32_wrap {
    ($name:ident, $core:ident) => {
        pub fn $name(x: f32) -> f32 { $core(x as f64) as f32 }
    };
}
f32_wrap!(core_sqrtf, core_sqrt);
f32_wrap!(core_sinf, core_sin);
f32_wrap!(core_cosf, core_cos);
f32_wrap!(core_tanf, core_tan);
f32_wrap!(core_expf, core_exp);
f32_wrap!(core_logf, core_log);
f32_wrap!(core_log2f, core_log2);
f32_wrap!(core_log10f, core_log10);
f32_wrap!(core_atanf, core_atan);
f32_wrap!(core_asinf, core_asin);
f32_wrap!(core_acosf, core_acos);
f32_wrap!(core_sinhf, core_sinh);
f32_wrap!(core_coshf, core_cosh);
f32_wrap!(core_tanhf, core_tanh);
f32_wrap!(core_cbrtf, core_cbrt);
f32_wrap!(core_floorf, core_floor);
f32_wrap!(core_ceilf, core_ceil);
f32_wrap!(core_truncf, core_trunc);
f32_wrap!(core_roundf, core_round);
f32_wrap!(core_fabsf, core_fabs);

pub fn core_powf(x: f32, y: f32) -> f32 { core_pow(x as f64, y as f64) as f32 }
pub fn core_atan2f(y: f32, x: f32) -> f32 { core_atan2(y as f64, x as f64) as f32 }
pub fn core_fmodf(x: f32, y: f32) -> f32 { core_fmod(x as f64, y as f64) as f32 }
pub fn core_hypotf(x: f32, y: f32) -> f32 { core_hypot(x as f64, y as f64) as f32 }
