// 超越函数（最终版）。纯计算、无 syscall ⇒ 宿主验证器 include! 对照真 libm。
//
// 关键修正（每一条都有验证器实测证据）：
//  1. sin_poly 符号全反 → 换 fdlibm __kernel_sin/cos 的系数与形式（带 y 尾项）
//  2. 归约精度不足（x-n·π/2 抵消）→ 换 fdlibm 三轮 rem_pio2（pio2_1/1t/2/2t/3/3t）
//  3. scale2 位模式错（2^1023 误当 2^1000）+ 未夹紧 k（曾致死循环）
//  4. expm1 缺大参数守卫；sinh/tanh 走 exp(x)-1 抵消 → 改 expm1 路径
//  5. log1p Horner 双重取负；log2 经 log·inv_ln2 多一次舍入 → 直接算
//  6. cbrt 位技巧初值偏差大 → 改 exp(log/3) 初值
//  7. atan 在 [0,1] 直用 Taylor（x→1 收敛极慢）→ 归约到 [0,tan(π/8)]
//  8. atan2 用 y>=0 判符号（-0.0 为真）→ 改符号位

use super::math_core::*;

const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INV_LN2: f64 = 1.44269504088896338700e+00;
const PI: f64 = 3.141592653589793;
const PIO2: f64 = 1.5707963267948966;

// ---- fdlibm 归约常数（**三组主项**，不是尾项；首版配错导致 sin/cos 完全错）----
const PIO2_1: f64 = 1.57079632673412561417e+00;
const PIO2_1T: f64 = 6.07710050650619224932e-11;
const PIO2_2: f64 = 6.07710050630396597660e-11;
const PIO2_2T: f64 = 2.02226624879595063154e-21;
const PIO2_3: f64 = 2.02226624871116645580e-21;
const PIO2_3T: f64 = 8.47842766036889956997e-32;
const INVPIO2: f64 = 6.36619772367581382433e-01;

// ---- fdlibm __kernel_sin/cos 系数 ----
const S1: f64 = -1.66666666666666324348e-01;
const S2: f64 = 8.33333333332248946124e-03;
const S3: f64 = -1.98412698298579493134e-04;
const S4: f64 = 2.75573137070700676789e-06;
const S5: f64 = -2.50507602534068634195e-08;
const S6: f64 = 1.58969099521155010221e-10;
const C1: f64 = 4.16666666666666019037e-02;
const C2: f64 = -1.38888888888741095749e-03;
const C3: f64 = 2.48015872894767294178e-05;
const C4: f64 = -2.75573143513906633035e-07;
const C5: f64 = 2.08757232129817482790e-09;
const C6: f64 = -1.13596475577881948265e-11;
const SPLIT: f64 = 134217729.0;

fn scale2(mut v: f64, mut k: i64) -> f64 {
    if k > 1024 { return v * f64::INFINITY; }
    if k < -1075 { return v * 0.0; }
    while k > 512 { v *= f64::from_bits(0x5ff0_0000_0000_0000); k -= 512; }
    while k < -512 { v *= f64::from_bits(0x1ff0_0000_0000_0000); k += 512; }
    v * f64::from_bits(((1023 + k) as u64) << 52)
}

// Dekker 分裂两积：返回 (p, err) 使 a*b = p + err（精确）。
fn two_prod(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    let e = SPLIT * a;
    let ahi = e - (e - a);
    let alo = a - ahi;
    let g = SPLIT * b;
    let bhi = g - (g - b);
    let blo = b - bhi;
    let err = ((ahi * bhi - p) + ahi * blo + alo * bhi) + alo * blo;
    (p, err)
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
    if x > 709.782712893384 { return f64::INFINITY; }
    if x < -745.1332191019411 { return -1.0; }
    if x.is_nan() { return x; }
    if core_fabs(x) < 1e-5 {
        let x2 = x * x;
        return x + x2 * (0.5 + x * (1.0 / 6.0 + x * (1.0 / 24.0 + x / 120.0)));
    }
    let k = (x * INV_LN2 + if x >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let kf = k as f64;
    let r = (x - kf * LN2_HI) - kf * LN2_LO;
    // expm1(r)，|r| ≤ 0.3466
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
    // **必须补这一项**：expm1(r)/r = 1 + r/2 + r²/6 + …，首版漏了 `+1.0`
    // ⇒ er = r·(0.5+…) 而非 r·(1+…)，expm1/sinh/tanh 全错。
    p = p * r + 1.0;
    let er = r * p;
    scale2(er, k) + (scale2(1.0, k) - 1.0)
}

fn log_reduce(x: f64) -> (f64, i64) {
    let mut bits = x.to_bits();
    let mut k = (((bits >> 52) & 0x7ff) as i64) - 1023;
    if k == -1023 {
        let y = x * f64::from_bits(0x4350_0000_0000_0000);
        bits = y.to_bits();
        k = (((bits >> 52) & 0x7ff) as i64) - 1023 - 54;
    }
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > 1.4142135623730951 { m *= 0.5; k += 1; }
    (m, k)
}

fn log_series(m: f64) -> f64 {
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut p = 1.0 / 25.0;
    let mut i = 23.0;
    while i >= 1.0 { p = p * s2 + 1.0 / i; i -= 2.0; }
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

pub fn core_log2(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    let (m, k) = log_reduce(x);
    // log2(m) 用双字：s 级数结果乘以 2/ln2 的 hi/lo，减少一次舍入
    let ls = log_series(m);
    let (p, e) = two_prod(ls, INV_LN2);
    k as f64 + (p + e)
}

pub fn core_log10(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    let (m, k) = log_reduce(x);
    let ls = log_series(m);
    let (p, e) = two_prod(ls, 4.34294481903251827652e-01);
    (k as f64) * 3.01029995663981195214e-01 + (p + e)
}

pub fn core_log1p(x: f64) -> f64 {
    // 阈值从 1e-2 放宽到 0.5：级数在 |x| ≤ 0.5 上仍收敛良好，
    // 而走 core_log(1+x) 会先形成 1+x（引入舍入）——实测 5 ulp 就是这条路径。
    if core_fabs(x) < 0.5 {
        let n = 40i32;
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

/// `pow`：整数指数反复平方（精确）；一般情形 `exp(y·log x)`，其中 `y·log x` 用 Dekker 双字。
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
        while n > 0 { if n & 1 == 1 { acc *= base; } base *= base; n >>= 1; }
        return if neg { 1.0 / acc } else { acc };
    }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return if y > 0.0 { 0.0 } else { f64::INFINITY }; }
    let lx = core_log(x);
    let (thi, tlo) = two_prod(y, lx);
    // exp(thi + tlo) = exp(thi)·exp(tlo) ≈ exp(thi)·(1+tlo)
    core_exp(thi) * (1.0 + tlo)
}

// ---- fdlibm 三轮归约：返回 (n mod 4, y0, y1)，r = y0 + y1 且 |r| ≤ π/4 ----
fn rem_pio2(x: f64) -> (i64, f64, f64) {
    let n = (x * INVPIO2 + if x >= 0.0 { 0.5 } else { -0.5 }) as i64;
    let fn_ = n as f64;
    let mut r = x - fn_ * PIO2_1;
    let mut w = fn_ * PIO2_1T;
    let mut y0 = r - w;
    let j = ((x.to_bits() >> 20) & 0x7ff) as i64;
    let high = ((y0.to_bits() >> 20) & 0x7ff) as i64;
    if (j - high).abs() > 16 {
        let t = r;
        w = fn_ * PIO2_2;
        r = t - w;
        w = fn_ * PIO2_2T - ((t - r) - w);
        y0 = r - w;
        let high2 = ((y0.to_bits() >> 20) & 0x7ff) as i64;
        if (j - high2).abs() > 49 {
            let t2 = r;
            w = fn_ * PIO2_3;
            r = t2 - w;
            w = fn_ * PIO2_3T - ((t2 - r) - w);
            y0 = r - w;
        }
    }
    let y1 = (r - y0) - w;
    (n & 3, y0, y1)
}

// fdlibm __kernel_sin(x, y, iy)
fn k_sin(x: f64, y: f64, iy: bool) -> f64 {
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    if !iy {
        x + v * (S1 + z * r)
    } else {
        x - ((z * (0.5 * y - v * r) - y) - v * S1)
    }
}

// fdlibm __kernel_cos(x, y)
fn k_cos(x: f64, y: f64) -> f64 {
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    // fdlibm __kernel_cos 原式。**注意**：`ix` 比较的是**高 32 位**（原 C 的 __HI），
    // 首版拿整个 64 位位模式去比 0x3FD33333，判据永远不成立 ⇒ 结果完全错。
    let ix = (x.to_bits() >> 32) & 0x7fff_ffff;
    if ix < 0x3FD3_3333 {
        1.0 - (0.5 * z - (z * r - x * y))
    } else {
        let qx = if ix > 0x3fe9_0000 {
            0.28125
        } else {
            f64::from_bits((ix - 0x0020_0000) << 32)
        };
        let hz = 0.5 * z - qx;
        let a = 1.0 - qx;
        a - (hz - (z * r - x * y))
    }
}

pub fn core_sin(x: f64) -> f64 {
    if !x.is_finite() { return f64::NAN; }
    if core_fabs(x) < 1e-8 { return x; }
    let (n, y0, y1) = rem_pio2(x);
    match n {
        0 => k_sin(y0, y1, true),
        1 => k_cos(y0, y1),
        2 => -k_sin(y0, y1, true),
        _ => -k_cos(y0, y1),
    }
}

pub fn core_cos(x: f64) -> f64 {
    if !x.is_finite() { return f64::NAN; }
    let (n, y0, y1) = rem_pio2(x);
    match n {
        0 => k_cos(y0, y1),
        1 => -k_sin(y0, y1, true),
        2 => -k_cos(y0, y1),
        _ => k_sin(y0, y1, true),
    }
}

pub fn core_tan(x: f64) -> f64 {
    if !x.is_finite() { return f64::NAN; }
    let (n, y0, y1) = rem_pio2(x);
    let t = k_tan(y0, y1, 1);
    if n & 1 == 0 { t } else { -1.0 / t }
}
// ---- fdlibm __kernel_tan（T[] 系数 + pio4 归约）----
const T0: f64 = 3.33333333333334091986e-01;
const T1: f64 = 1.33333333333201242699e-01;
const T2: f64 = 5.39682539762260521377e-02;
const T3: f64 = 2.18694882948595424599e-02;
const T4: f64 = 8.86323982359930005737e-03;
const T5: f64 = 3.59207910759131235356e-03;
const T6: f64 = 1.45620945432529025516e-03;
const T7: f64 = 5.88041240820264096874e-04;
const T8: f64 = 2.46463134818469906812e-04;
const T9: f64 = 7.81794442939557092300e-05;
const T10: f64 = 7.14072491382608190305e-05;
const T11: f64 = -1.85586374855275456654e-05;
const T12: f64 = 2.59073051863633712884e-05;
const PIO4: f64 = 7.85398163397448278999e-01;
const PIO4LO: f64 = 3.06161699786838301793e-17;

fn k_tan(mut x: f64, mut y: f64, iy: i64) -> f64 {
    let hx = (x.to_bits() >> 32) as u32;
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3FE5_9428 {
        if (hx as i32) < 0 { x = -x; y = -y; }
        let z = PIO4 - x;
        let w = PIO4LO - y;
        x = z + w;
        y = 0.0;
    }
    let z = x * x;
    let w = z * z;
    let r = T1 + w * (T3 + w * (T5 + w * (T7 + w * (T9 + w * T11))));
    let v = z * (T2 + w * (T4 + w * (T6 + w * (T8 + w * (T10 + w * T12)))));
    let s = z * x;
    let mut r2 = y + z * (s * (r + v) + y);
    r2 += T0 * s;
    let w2 = x + r2;
    if ix >= 0x3FE5_9428 {
        let vv = iy as f64;
        let sign = 1.0 - (((hx >> 30) & 2) as f64);
        return sign * (vv - 2.0 * (x - (w2 * w2 / (w2 + vv) - r2)));
    }
    if iy == 1 { w2 } else { -1.0 / (x + r2) }
}

// ---- fdlibm asin 的 pS/qS 系数 ----
const PS0: f64 = 1.66666666666666657415e-01;
const PS1: f64 = -3.25565818622400915405e-01;
const PS2: f64 = 2.01212532134862925881e-01;
const PS3: f64 = -4.00555345006794114027e-02;
const PS4: f64 = 7.91534994289814532176e-04;
const PS5: f64 = 3.47933107596021167570e-05;
const QS1: f64 = -2.40339491173441421878e+00;
const QS2: f64 = 2.02094576023350569471e+00;
const QS3: f64 = -6.88283971605453293030e-01;
const QS4: f64 = 7.70381505559019352791e-02;
const PIO2_HI: f64 = 1.57079632679489655800e+00;
const PIO2_LO: f64 = 6.12323399573676603587e-17;

/// `asin`：fdlibm 路径（|x|<0.5 有理逼近；否则 sqrt + 双字补偿）。
pub fn core_asin_fd(x: f64) -> f64 {
    let hx = (x.to_bits() >> 32) as u32;
    let ix = hx & 0x7fff_ffff;
    if ix < 0x3FDC_0000 {
        if ix < 0x3E40_0000 { return x; }
        let t = x * x;
        let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
        let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
        let w = p / q;
        return x + x * w;
    }
    let w = 1.0 - core_fabs(x);
    let t = w * 0.5;
    let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
    let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
    let s = core_sqrt(t);
    let out = if ix >= 0x3FEF_3333 {
        let w2 = p / q;
        PIO2_HI - (2.0 * (s + s * w2) - PIO2_LO)
    } else {
        let w2 = f64::from_bits(s.to_bits() & 0xffff_ffff_0000_0000);
        let c = (t - w2 * w2) / (s + w2);
        let r = p / q;
        let p2 = 2.0 * s * r - (PIO2_LO - 2.0 * c);
        let q2 = PIO4 - 2.0 * w2;
        PIO4 - (p2 - q2)
    };
    if (hx as i32) > 0 { out } else { -out }
}

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
    if ax <= 1.0 { core_copysign(atan_unit(ax), x) }
    else { core_copysign(PIO2 - atan_unit(1.0 / ax), x) }
}

pub fn core_atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() { return f64::NAN; }
    if x > 0.0 { return core_atan(y / x); }
    if x < 0.0 {
        let a = core_atan(y / x);
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
    core_asin_fd(x)
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
    // sinh = (e^x − e^−x)/2 = (expm1(x) − expm1(−x))/2 —— 首版误写成 `+`。
    core_copysign(0.5 * (core_expm1(ax) - core_expm1(-ax)), x)
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
    let t = core_expm1(2.0 * ax);
    core_copysign(t / (t + 2.0), x)
}

pub fn core_cbrt(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() { return x; }
    let ax = core_fabs(x);
    let mut y = core_exp(core_log(ax) / 3.0);
    for _ in 0..3 { y = y - (y - ax / (y * y)) / 3.0; }
    let mut best = y;
    let mut best_err = core_fabs(y * y * y - ax);
    for d in [-4i64, -3, -2, -1, 1, 2, 3, 4] {
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
    let t = 1.0 + r * r;
    let s = core_sqrt(t);
    // 末次补偿：在 cand 与相邻可表示值中选更接近 t 者。
    // **用归一化比较（cand/hi 与 sqrt(t) 同量级）**，不用 hi² 那类会溢出的量。
    let cand = hi * s;
    let s2 = f64::from_bits(s.to_bits() + 1);
    if (s2 * s2 - t).abs() < (s * s - t).abs() { hi * s2 } else { cand }
}

pub fn core_nearbyint(x: f64) -> f64 {
    let t = core_trunc(x);
    let d = x - t;
    let ad = core_fabs(d);
    if ad < 0.5 { t } else if ad > 0.5 { t + core_copysign(1.0, x) }
    else if (t as i64) % 2 == 0 { t } else { t + core_copysign(1.0, x) }
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
    ($name:ident, $core:ident) => { pub fn $name(x: f32) -> f32 { $core(x as f64) as f32 } };
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
