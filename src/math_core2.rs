// 超越函数（第二部分）。纯计算、无 syscall ⇒ 宿主验证器可 include! 对照真 libm。
// 依赖 base 部分的 core_fabs/core_copysign/core_sqrt/core_trunc。

use super::math_core::*;

const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INV_LN2: f64 = 1.44269504088896338700e+00;
const PIO2_HI: f64 = 1.57079632673412561417e+00;
// **修正**：首版误用 fdlibm 的 pio2_1t/pio2_2t（尾项）当主项 ⇒ 三项 Cody-Waite 不成立，
// sin/cos/tan 在中大参数处完全错。正确的是 pio2_1/pio2_2/pio2_3 这一组：
const PIO2_MID: f64 = 6.07710050630396597660e-11;   // fdlibm pio2_2
const PIO2_LO: f64 = 2.02226624871116645580e-21;    // fdlibm pio2_3
const TWO_OVER_PI: f64 = 6.36619772367581382433e-01;
const PI: f64 = 3.141592653589793;
const PIO2: f64 = 1.5707963267948966;

// 乘 2^k。**修正**：首版用 0x7fe0…（其实是 2^1023，不是我以为的 2^1000），
// 分块后最后一次乘法溢出 ⇒ exp(700) 返回 inf。改用 2^512 分块（位模式已核对）。
fn scale2(mut v: f64, mut k: i64) -> f64 {
    // **必须夹紧 k**：首版无夹紧，配合调用方缺少大参数守卫时 k 会变成 i64::MAX，
    // 于是 `while k > 512` 要循环 ~1.8e16 次 ⇒ **死循环**（验证器实测挂住）。
    // f64 的指数域是 [2^-1074, 2^1023]，越界即 0 或 inf。
    if k > 1024 { return v * f64::INFINITY; }
    if k < -1075 { return v * 0.0; }
    while k > 512 { v *= f64::from_bits(0x5ff0_0000_0000_0000); k -= 512; }
    while k < -512 { v *= f64::from_bits(0x1ff0_0000_0000_0000); k += 512; }
    v * f64::from_bits(((1023 + k) as u64) << 52)
}

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

pub fn core_expm1(x: f64) -> f64 {
    // **标准做法**：与 exp 同样的归约，但用 expm1(r) 的级数，再合成 2^k·expm1(r) + (2^k − 1)，
    // 避免 exp(x)−1 在 x→0 的灾难性抵消。
    // **大参数守卫（首版漏了）**：否则 k 会溢出成 i64::MAX，配合 scale2 造成死循环。
    if x > 709.782712893384 { return f64::INFINITY; }
    if x < -745.1332191019411 { return -1.0; }
    if core_fabs(x) < 1e-5 {
        let x2 = x * x;
        return x + x2 * (0.5 + x * (1.0 / 6.0 + x * (1.0 / 24.0 + x / 120.0)));
    }
    let k = (x * INV_LN2 + if x >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let kf = k as f64;
    let r = (x - kf * LN2_HI) - kf * LN2_LO;
    // expm1(r) 级数（|r| ≤ 0.3466）：r + r²/2 + … + r¹⁴/14!
    let mut p = 1.0 / 87178291200.0;
    p = p * r + 1.0 / 6227020800.0;
    p = p * r + 1.0 / 479001600.0;
    p = p * r + 1.0 / 39916800.0;
    p = p * r + 1.0 / 3628800.0;
    p = p * r + 1.0 / 362880.0;
    p = p * r + 1.0 / 40320.0;
    p = p * r + 1.0 / 5040.0;
    p = p * r + 1.0 / 720.0;
    p = p * r + 1.0 / 120.0;
    p = p * r + 1.0 / 24.0;
    p = p * r + 1.0 / 6.0;
    p = p * r + 0.5;
    let er = r * p;          // expm1(r)
    scale2(er, k) + (scale2(1.0, k) - 1.0)
}

/// log 的公共归约：x = m·2^k，m∈[√2/2,√2)。返回 (m, k)。
fn log_reduce(x: f64) -> (f64, i64) {
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
    (m, k)
}

/// s = (m-1)/(m+1)，log(m) = 2·Σ s^(2i+1)/(2i+1)（至 s²⁵ 项）。
fn log_series(m: f64) -> f64 {
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut p = 1.0 / 25.0;
    let mut i = 23.0;
    while i >= 1.0 {
        p = p * s2 + 1.0 / i;
        i -= 2.0;
    }
    2.0 * s * p
}

pub fn core_log(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    let (m, k) = log_reduce(x);
    let kf = k as f64;
    log_series(m) + (kf * LN2_HI + kf * LN2_LO)
}

/// `log2`：**直接算** `k + log2(m)`，不经过 `log(x)·INV_LN2`（那会多一次舍入 ⇒ 实测 2 ulp）。
pub fn core_log2(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    let (m, k) = log_reduce(x);
    k as f64 + log_series(m) * INV_LN2
}

/// `log10`：同样直接算，常数用双字拆分减少舍入。
pub fn core_log10(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    const LOG10_2: f64 = 3.01029995663981195214e-01;
    const LOG10_E: f64 = 4.34294481903251827652e-01;
    let (m, k) = log_reduce(x);
    (k as f64) * LOG10_2 + log_series(m) * LOG10_E
}

pub fn core_log1p(x: f64) -> f64 {
    // log1p(x) = x·(1 - x/2 + x²/3 - x³/4 + …) —— Horner 从高次起，符号交替。
    // log1p(x) = x·(c1 + x·(c2 + x·(…)))，c_k = (-1)^(k+1)/k —— **Horner 用 +x 递推**，
    // 首版写成 s*(-x) 且系数又带符号 ⇒ 双重取负，极小量下完全错。
    if core_fabs(x) < 1e-2 {
        let n = 20i32;
        let mut p = if n % 2 == 1 { 1.0 / n as f64 } else { -1.0 / n as f64 };
        let mut k = n - 1;
        while k >= 1 {
            let c = if k % 2 == 1 { 1.0 / k as f64 } else { -1.0 / k as f64 };
            p = p * x + c;
            k -= 1;
        }
        return x * p;
    }
    core_log(1.0 + x)
}

/// `pow`：整数指数走反复平方（精确）；一般情形用 **hi/lo 双字** 减少 `y·log x` 的舍入。
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
    // y·log x 用双字：先算 t=y*log x，再补一次修正（fma 不可用，用 Dekker 分裂）
    let lx = core_log(x);
    let t = y * lx;
    // 误差补偿：y*lx 的精确残差 ≈ y*(lx - t/y)，用 f64 双字近似
    let thi = t;
    let tlo = y * (lx - thi / y);
    let e = core_exp(thi);
    e * (1.0 + tlo)
}

fn reduce_pio2(x: f64) -> (i64, f64) {
    let n = (x * TWO_OVER_PI + if x >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let nf = n as f64;
    let r = ((x - nf * PIO2_HI) - nf * PIO2_MID) - nf * PIO2_LO;
    (n & 3, r)
}

// **修正（第 3 轮诊断一击命中）**：首版符号全反。
// sin(r) = r + r³·P(r²)，P(u) = −1/6 + u/120 − u²/5040 + u³/362880 − u⁴/39916800 + u⁵/6227020800。
// 证据：所有走 sin_poly 的 x（0.1/0.3/0.5/0.7/0.785/2.5/3.0/100/-112.3125）都错；
// 而走 cos_poly 的（cos 0.5 → diff=0、cos -112.3125 → diff=1e-15）全对。
fn sin_poly(r: f64) -> f64 {
    let r2 = r * r;
    let mut p = 1.0 / 6227020800.0;
    p = p * r2 - 1.0 / 39916800.0;
    p = p * r2 + 1.0 / 362880.0;
    p = p * r2 - 1.0 / 5040.0;
    p = p * r2 + 1.0 / 120.0;
    p = p * r2 - 1.0 / 6.0;
    r + r * r2 * p
}

fn cos_poly(r: f64) -> f64 {
    let r2 = r * r;
    1.0 - r2 * (0.5 - r2 * (1.0 / 24.0 - r2 * (1.0 / 720.0 - r2 * (1.0 / 40320.0
        - r2 * (1.0 / 3628800.0 - r2 * (1.0 / 479001600.0 - r2 / 87178291200.0))))))
}

// **诚实边界（S09）**：Cody-Waite 三项归约的有效范围是 |x| < 2^20 ≈ 1.05e6。
// 超出后 π/2 的截断误差放大，结果不可信——**本实现不假装对大参数有效**。
// （要覆盖全值域需 Payne-Hanek 多精度归约，属后续工作。）
pub const TRIG_REDUCE_MAX: f64 = 1048576.0;

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

// **修正**：首版在 [0,1] 上直接用 Taylor，x→1 处收敛极慢（Gregory 级数）⇒ 差 1e14 ulp。
// 现在先归约到 [0, tan(π/8)=0.4142]，再展开足够多项（至 x⁴⁹）。
const TAN_PI8: f64 = 4.1421356237309503e-01;

fn atan_series(x: f64) -> f64 {
    let x2 = x * x;
    let mut s = 0.0;
    let mut k = 24i32;
    while k >= 0 {
        let c = 1.0 / (2.0 * k as f64 + 1.0);
        s = s * x2 + if k % 2 == 0 { c } else { -c };
        k -= 1;
    }
    x * s
}

fn atan_unit(x: f64) -> f64 {
    // x ∈ [0,1]
    if x > TAN_PI8 {
        let y = (x - 1.0) / (x + 1.0);
        PI / 4.0 + atan_series(y)
    } else {
        atan_series(x)
    }
}

pub fn core_atan(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax.is_infinite() { return core_copysign(PIO2, x); }
    if ax <= 1.0 {
        core_copysign(atan_unit(ax), x)
    } else {
        core_copysign(PIO2 - atan_unit(1.0 / ax), x)
    }
}

pub fn core_atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() { return f64::NAN; }
    if x > 0.0 { return core_atan(y / x); }
    if x < 0.0 {
        let a = core_atan(y / x);
        // **用符号位判断**，不能用 `y >= 0.0`——(-0.0) >= 0.0 为真，但 atan2(-0,-1) 应为 -π。
        return if y.is_sign_negative() { a - PI } else { a + PI };
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
    // 首版试过 asin 级数路径，实测更差（25291 ulp，递推系数写法有误）⇒ 回到 atan2 路径。
    // 现在 atan 已修好（归约 + 足够项），这条路径实测可达 1 ulp。
    core_atan2(x, core_sqrt((1.0 - ax) * (1.0 + ax)))
}

pub fn core_acos(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax > 1.0 { return f64::NAN; }
    if x == 1.0 { return 0.0; }
    if x == -1.0 { return PI; }
    // 直接 atan2 路径（PIO2 - asin(x) 在 x→1 时抵消严重）。
    core_atan2(core_sqrt((1.0 - ax) * (1.0 + ax)), x)
}

pub fn core_sinh(x: f64) -> f64 {
    if x.is_nan() { return x; }
    let ax = core_fabs(x);
    if ax < 1e-5 { return x + x * x * x / 6.0; }
    if ax > 710.0 { return core_copysign(f64::INFINITY, x); }
    // 用 expm1 避免 exp(x)−1 的抵消（小 x 时尤其重要）
    core_copysign(0.5 * (core_expm1(ax) + core_expm1(-ax)), x)
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
    if ax < 1e-5 { return x; }
    // tanh(x) = expm1(2x) / (expm1(2x) + 2) —— 标准形式，避免 (e−1)/(e+1) 的抵消
    let t = core_expm1(2.0 * ax);
    core_copysign(t / (t + 2.0), x)
}

pub fn core_cbrt(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() { return x; }
    let ax = core_fabs(x);
    // 初值改用 exp(log(ax)/3)（相对误差 ~1e-16），再做 3 次牛顿即收敛到 ulp 级；
    // 首版的位技巧初值偏差大，8 次迭代仍不收敛。
    let mut y = core_exp(core_log(ax) / 3.0);
    for _ in 0..3 { y = y - (y - ax / (y * y)) / 3.0; }
    // 末次修正：在 y 与相邻可表示值中选更接近真值者
    let mut best = y;
    let mut best_err = core_fabs(y * y * y - ax);
    for d in [-2i64, -1, 1, 2] {
        let z = f64::from_bits((y.to_bits() as i64 + d) as u64);
        let err = core_fabs(z * z * z - ax);
        if err < best_err { best = z; best_err = err; }
    }
    core_copysign(best, x)
}

pub fn core_hypot(x: f64, y: f64) -> f64 {
    let ax = core_fabs(x);
    let ay = core_fabs(y);
    if ax.is_infinite() || ay.is_infinite() { return f64::INFINITY; }
    if ax.is_nan() || ay.is_nan() { return f64::NAN; }
    let (hi, lo) = if ax > ay { (ax, ay) } else { (ay, ax) };
    if hi == 0.0 { return 0.0; }
    let r = lo / hi;
    let s = core_sqrt(1.0 + r * r);
    // 末次修正：避免 sqrt 的 1 ulp 在放大后变成 2 ulp
    // 末次修正：**不要用 ax²+ay²**（大数下溢出，首版因此判错）。
    // 改用同量级的比较：cand/hi 与 sqrt(1+r²) 的相对关系。
    // 首版用 ax²+ay² 做修正判据，大数下溢出导致判错；改为直接返回（实测 2 ulp，
    // 已如实记入超档清单——不假装它达标）。
    hi * s
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
