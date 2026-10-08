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

// ---- double-double（Dekker）辅助：log2 的级数用它把内部误差压到 ~2^-106 ----
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

fn quick_two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    (s, b - (s - a))
}

fn dd_add(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (s1, s2) = two_sum(a.0, b.0);
    let (t1, t2) = two_sum(a.1, b.1);
    let (s1, s2) = quick_two_sum(s1, s2 + t1);
    quick_two_sum(s1, s2 + t2)
}

fn dd_mul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (p1, p2) = two_prod(a.0, b.0);
    quick_two_sum(p1, p2 + (a.0 * b.1 + a.1 * b.0))
}

/// 小整数 d 的**双字倒数**：hi = fl(1/d)，lo = (1 - hi·d)/d（精确残差）
fn dd_recip_int(d: f64) -> (f64, f64) {
    let hi = 1.0 / d;
    let (p, e) = two_prod(hi, d);
    (hi, ((1.0 - p) - e) / d)
}

// ---- fdlibm 的 GET/SET_{HIGH,LOW}_WORD ----
fn set_high_word(v: f64, hw: u32) -> f64 {
    f64::from_bits(((hw as u64) << 32) | (v.to_bits() & 0xffff_ffff))
}
fn set_low_word(v: f64, lw: u32) -> f64 {
    f64::from_bits((v.to_bits() & 0xffff_ffff_0000_0000) | (lw as u64))
}
fn get_high_word(v: f64) -> u32 { (v.to_bits() >> 32) as u32 }
fn get_low_word(v: f64) -> u32 { v.to_bits() as u32 }

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

/// `ln(m)` 的双字版本（m ∈ [√2/2, √2]，即 `log_reduce` 的输出）。
/// 返回 (hi, lo) 使 hi+lo ≈ ln(m)，内部误差 ~2^-106。
fn log_series_dd(m: f64) -> (f64, f64) {
    // f = m-1 在 m∈[0.5,2] 上精确（Sterbenz）；分母 1+m 用 two_sum 精确拆成 (d_hi,d_lo)
    let f = m - 1.0;
    let (d_hi, d_lo) = two_sum(m, 1.0);
    let s_hi = f / d_hi;
    let (p, e) = two_prod(s_hi, d_hi);
    // 残差必须把 d_lo 也算进去：否则分母那一次舍入会带进 2^-53 相对误差（≈1 ulp）
    let s_lo = (((f - p) - e) - s_hi * d_lo) / d_hi;
    let s = (s_hi, s_lo);
    let s2 = dd_mul(s, s);
    let mut term = s;
    let mut sum = s;
    let mut i = 3i32;
    // |s| ≤ 0.1716 ⇒ s² ≤ 0.0295；算到 s^45 ≈ 2^-117，尾部可忽略
    while i <= 45 {
        term = dd_mul(term, s2);
        sum = dd_add(sum, dd_mul(term, dd_recip_int(i as f64)));
        i += 2;
    }
    (sum.0 * 2.0, sum.1 * 2.0) // ln(m) = 2·(s + s³/3 + s⁵/5 + …)
}

pub fn core_log2(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    let (m, k) = log_reduce(x);
    let ln = log_series_dd(m);
    // 1/ln2 的双字：hi = fl(1/ln2)，lo = (1 - hi·ln2)/ln2
    let (p, e) = two_prod(INV_LN2, LN2_HI);
    let inv_tail = (((1.0 - p) - e) - INV_LN2 * LN2_LO) / LN2_HI;
    let (qh, ql) = dd_mul(ln, (INV_LN2, inv_tail));
    // k 是精确整数；用 two_sum 保证 k + q 只舍入一次
    let (t1, t2) = two_sum(k as f64, qh);
    t1 + (t2 + ql)
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

// ---- fdlibm e_pow.c 常量（前缀 PW_，避免与别处重名）----
const PW_BP: [f64; 2] = [1.0, 1.5];
const PW_DP_H: [f64; 2] = [0.0, 5.84962487220764160156e-01];
const PW_DP_L: [f64; 2] = [0.0, 1.35003920212974897128e-08];
const PW_TWO53: f64 = 9007199254740992.0;
const PW_HUGE: f64 = 1.0e300;
const PW_TINY: f64 = 1.0e-300;
const PW_L1: f64 = 5.99999999999994648725e-01;
const PW_L2: f64 = 4.28571428578550184252e-01;
const PW_L3: f64 = 3.33333329818377432918e-01;
const PW_L4: f64 = 2.72728123808534006489e-01;
const PW_L5: f64 = 2.30660745775561754067e-01;
const PW_L6: f64 = 2.06975017800338417784e-01;
const PW_P1: f64 = 1.66666666666666019037e-01;
const PW_P2: f64 = -2.77777777770155933842e-03;
const PW_P3: f64 = 6.61375632143793436117e-05;
const PW_P4: f64 = -1.65339022054652515390e-06;
const PW_P5: f64 = 4.13813679705723846039e-08;
const PW_LG2: f64 = 6.93147180559945286227e-01;
const PW_LG2_H: f64 = 6.93147182464599609375e-01;
const PW_LG2_L: f64 = -1.90465429995776804525e-09;
const PW_OVT: f64 = 8.0085662595372944372e-17;
const PW_CP: f64 = 9.61796693925975554329e-01;
const PW_CP_H: f64 = 9.61796700954437255859e-01;
const PW_CP_L: f64 = -7.02846165095275826516e-09;
const PW_IVLN2: f64 = 1.44269504088896338700e+00;
const PW_IVLN2_H: f64 = 1.44269502162933349609e+00;
const PW_IVLN2_L: f64 = 1.92596299112661746887e-08;

/// `pow`：fdlibm e_pow.c 忠实移植（"nearly rounded"，实测 ≤1 ulp）。
///
/// 旧实现 `exp(y·log x)` 实测 21 ulp：log(x) 只有 1 ulp，乘 y 后指数误差放大 |y| 倍，
/// exp 再把它 1:1 变成相对误差 —— 在 (1e8, 1.5) 处正好 ~21 ulp。
/// fdlibm 的做法是**全程多精度模拟**：log2(x) 拆成 t1+t2（t1 低 32 位清零），
/// y·log2(x) 拆成 y1+y2 做双字乘法，最后 2^(p_h+p_l) 用 lg2 的 hi/lo 补偿。
pub fn core_pow(x: f64, y: f64) -> f64 {
    let hx = get_high_word(x);
    let lx = get_low_word(x);
    let hy = get_high_word(y);
    let ly = get_low_word(y);
    let ix = hx & 0x7fff_ffff;
    let iy = hy & 0x7fff_ffff;

    if (iy | ly) == 0 { return 1.0; }                       // y == 0
    if hx == 0x3ff0_0000 && lx == 0 { return 1.0; }         // x == 1
    if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && lx != 0)
        || iy > 0x7ff0_0000 || (iy == 0x7ff0_0000 && ly != 0) {
        return (x + 0.0) + (y + 0.0);
    }

    // y 是否为整数（x<0 时需要）：0=否，1=奇，2=偶
    let mut yisint: i32 = 0;
    if hx & 0x8000_0000 != 0 {
        if iy >= 0x4340_0000 {
            yisint = 2;
        } else if iy >= 0x3ff0_0000 {
            let k = ((iy >> 20) as i32) - 0x3ff;
            if k > 20 {
                let j = ly >> (52 - k);
                if (j << (52 - k)) == ly { yisint = 2 - ((j & 1) as i32); }
            } else if ly == 0 {
                let j = iy >> (20 - k);
                if (j << (20 - k)) == iy { yisint = 2 - ((j & 1) as i32); }
            }
        }
    }

    // y 的特殊值
    if ly == 0 {
        if iy == 0x7ff0_0000 {
            if ((ix.wrapping_sub(0x3ff0_0000)) | lx) == 0 { return 1.0; }
            if ix >= 0x3ff0_0000 {
                return if hy < 0x8000_0000 { y } else { 0.0 };
            }
            return if hy >= 0x8000_0000 { -y } else { 0.0 };
        }
        if iy == 0x3ff0_0000 { return if hy >= 0x8000_0000 { 1.0 / x } else { x }; }
        if hy == 0x4000_0000 { return x * x; }
        if hy == 0x4008_0000 { return x * x * x; }
        if hy == 0x4010_0000 { let u = x * x; return u * u; }
        if hy == 0x3fe0_0000 && hx < 0x8000_0000 { return core_sqrt(x); }
    }

    let ax0 = core_fabs(x);
    // x 的特殊值
    if lx == 0 && (ix == 0x7ff0_0000 || ix == 0 || ix == 0x3ff0_0000) {
        let mut z = ax0;
        if hy >= 0x8000_0000 { z = 1.0 / z; }
        if hx >= 0x8000_0000 {
            if ((ix.wrapping_sub(0x3ff0_0000)) | (yisint as u32)) == 0 {
                z = (z - z) / (z - z); // (-1)^非整数 = NaN
            } else if yisint == 1 {
                z = -z;
            }
        }
        return z;
    }

    let n0 = ((hx >> 31) as i32) - 1;
    if (n0 | yisint) == 0 { return (x - x) / (x - x); } // (x<0)^非整数
    let mut s = 1.0f64;
    if (n0 | (yisint - 1)) == 0 { s = -1.0; }

    let t1: f64;
    let t2: f64;
    if iy > 0x41e0_0000 {
        // |y| > 2^31
        if iy > 0x43f0_0000 {
            if ix <= 0x3fef_ffff {
                return if hy >= 0x8000_0000 { PW_HUGE * PW_HUGE } else { PW_TINY * PW_TINY };
            }
            if ix >= 0x3ff0_0000 {
                return if hy < 0x8000_0000 { PW_HUGE * PW_HUGE } else { PW_TINY * PW_TINY };
            }
        }
        if ix < 0x3fef_ffff {
            return if hy >= 0x8000_0000 { s * PW_HUGE * PW_HUGE } else { s * PW_TINY * PW_TINY };
        }
        if ix > 0x3ff0_0000 {
            return if hy < 0x8000_0000 { s * PW_HUGE * PW_HUGE } else { s * PW_TINY * PW_TINY };
        }
        // |1-x| 很小，log(x) 用 x-x²/2+x³/3-x⁴/4
        let t = ax0 - 1.0;
        let w = (t * t) * (0.5 - t * (0.3333333333333333333333 - t * 0.25));
        let u = PW_IVLN2_H * t;
        let v = t * PW_IVLN2_L - w * PW_IVLN2;
        t1 = set_low_word(u + v, 0);
        t2 = v - (t1 - u);
    } else {
        let mut ax = ax0;
        let mut n = 0i32;
        let mut ixm = ix;
        if ixm < 0x0010_0000 { ax *= PW_TWO53; n -= 53; ixm = get_high_word(ax); }
        n += ((ixm >> 20) as i32) - 0x3ff;
        let j = ixm & 0x000f_ffff;
        ixm = j | 0x3ff0_0000;
        let k: usize;
        if j <= 0x3988E { k = 0; }
        else if j < 0xBB67A { k = 1; }
        else { k = 0; n += 1; ixm -= 0x0010_0000; }
        ax = set_high_word(ax, ixm);
        // ss = s_h + s_l = (x-1)/(x+1) 或 (x-1.5)/(x+1.5)
        let u = ax - PW_BP[k];
        let v = 1.0 / (ax + PW_BP[k]);
        let ss = u * v;
        let s_h = set_low_word(ss, 0);
        let t_h0 = set_high_word(0.0, ((ixm >> 1) | 0x2000_0000) + 0x0008_0000 + ((k as u32) << 18));
        let t_l0 = ax - (t_h0 - PW_BP[k]);
        let s_l = v * ((u - s_h * t_h0) - s_h * t_l0);
        // log(ax)
        let s2 = ss * ss;
        let mut r = s2 * s2 * (PW_L1 + s2 * (PW_L2 + s2 * (PW_L3 + s2 * (PW_L4 + s2 * (PW_L5 + s2 * PW_L6)))));
        r += s_l * (s_h + ss);
        let s2b = s_h * s_h;
        let t_h = set_low_word(3.0 + s2b + r, 0);
        let t_l = r - ((t_h - 3.0) - s2b);
        let u2 = s_h * t_h;
        let v2 = s_l * t_h + t_l * ss;
        let p_h = set_low_word(u2 + v2, 0);
        let p_l = v2 - (p_h - u2);
        let z_h = PW_CP_H * p_h;
        let z_l = PW_CP_L * p_h + p_l * PW_CP + PW_DP_L[k];
        // log2(ax) = n + dp_h + z_h + z_l
        let t = n as f64;
        t1 = set_low_word(((z_h + z_l) + PW_DP_H[k]) + t, 0);
        t2 = z_l - (((t1 - t) - PW_DP_H[k]) - z_h);
    }

    // (y1+y2)*(t1+t2)
    let y1 = set_low_word(y, 0);
    let p_l = (y - y1) * t1 + y * t2;
    let mut p_h = y1 * t1;
    let mut z = p_l + p_h;
    let jz = get_high_word(z);
    let iz = get_low_word(z);
    let jzs = jz as i32;
    if jzs >= 0x4090_0000 {
        if ((jz.wrapping_sub(0x4090_0000)) | iz) != 0 { return s * PW_HUGE * PW_HUGE; }
        if p_l + PW_OVT > z - p_h { return s * PW_HUGE * PW_HUGE; }
    } else if (jz & 0x7fff_ffff) >= 0x4090_cc00 {
        if ((jz.wrapping_sub(0xc090_cc00)) | iz) != 0 { return s * PW_TINY * PW_TINY; }
        if p_l <= z - p_h { return s * PW_TINY * PW_TINY; }
    }

    // 2^(p_h+p_l)
    let i2 = (jz & 0x7fff_ffff) as i32;
    let mut k2 = (i2 >> 20) - 0x3ff;
    let mut n2 = 0i32;
    if i2 > 0x3fe0_0000 {
        n2 = jzs.wrapping_add((0x0010_0000u32 >> ((k2 + 1) & 31)) as i32);
        k2 = ((((n2 as u32) & 0x7fff_ffff) >> 20) as i32) - 0x3ff;
        let tt = set_high_word(0.0, (n2 as u32) & !(0x000f_ffffu32 >> (k2 & 31)));
        n2 = ((((n2 as u32) & 0x000f_ffff) | 0x0010_0000) >> ((20 - k2) & 31)) as i32;
        if jzs < 0 { n2 = -n2; }
        p_h -= tt;
    }
    let t = set_low_word(p_l + p_h, 0);
    let u = t * PW_LG2_H;
    let v = (p_l - (t - p_h)) * PW_LG2 + t * PW_LG2_L;
    z = u + v;
    let w = v - (z - u);
    let t2b = z * z;
    let t1b = z - t2b * (PW_P1 + t2b * (PW_P2 + t2b * (PW_P3 + t2b * (PW_P4 + t2b * PW_P5))));
    let r = (z * t1b) / (t1b - 2.0) - (w + z * w);
    z = 1.0 - (r - z);
    let jj = (get_high_word(z) as i32).wrapping_add(n2 << 20);
    if (jj >> 20) <= 0 {
        z = scale2(z, n2 as i64); // 次正规输出
    } else {
        z = set_high_word(z, jj as u32);
    }
    s * z
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
    // fdlibm：奇数象限**直接**调 k_tan(...,-1) 走精确倒数分支，
    // 而不是先算 tan 再在 double 里取负倒数（后者多一次舍入 ⇒ 2 ulp）。
    k_tan(y0, y1, if n & 1 == 0 { 1 } else { -1 })
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
    if iy == 1 {
        w2
    } else {
        // fdlibm 的**精确倒数**：直接 -1.0/(x+r) 只到 2 ulp —— 这正是 tan 之前 2 ulp 的来源。
        let zz = set_low_word(w2, 0);
        let vv = r2 - (zz - x); // zz + vv = r2 + x
        let a = -1.0 / w2;
        let t = set_low_word(a, 0);
        let s = 1.0 + t * zz;
        t + a * (s + t * vv)
    }
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

/// `asin`：fdlibm e_asin.c 的忠实移植（|x|<0.5 有理逼近；否则 pi/2-2·asin(√((1-|x|)/2))）。
///
/// 首版移植**两处错**：(1) 阈值写成 0x3FDC0000（应为 0x3FE00000，即 0.5），
/// 于是 |x|∈[0.4375,0.5) 被送进 s 路径，而 R(z) 的 Remez 区间只保证 z ≤ 0.25；
/// (2) 漏了 |x| ≥ 1 的分支。两者合起来实测 10 ulp，于是当时误判"移植更差"而回退。
pub fn core_asin_fd(x: f64) -> f64 {
    let hx = (x.to_bits() >> 32) as u32;
    let lx = x.to_bits() as u32;
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        // |x| >= 1
        if ((ix.wrapping_sub(0x3ff0_0000)) | lx) == 0 {
            return x * PIO2_HI + x * PIO2_LO;
        }
        return (x - x) / (x - x);
    }
    if ix < 0x3fe0_0000 {
        // |x| < 0.5
        if ix < 0x3e50_0000 { return x; }
        let t = x * x;
        let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
        let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
        let w = p / q;
        return x + x * w;
    }
    // 1 > |x| >= 0.5
    let w = 1.0 - core_fabs(x);
    let t = w * 0.5;
    let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
    let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
    let s = core_sqrt(t);
    let out = if ix >= 0x3FEF_3333 {
        // |x| > 0.975
        let w2 = p / q;
        PIO2_HI - (2.0 * (s + s * w2) - PIO2_LO)
    } else {
        let w2 = set_low_word(s, 0);
        let c = (t - w2 * w2) / (s + w2); // c = sqrt(t) - f
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

// ---- fdlibm s_cbrt.c（Bruce D. Evans 版，误差 < 0.667 ulp）----
const CBRT_B1: u32 = 715094163; // (1023-1023/3-0.03306235651)*2^20
const CBRT_B2: u32 = 696219795; // (1023-1023/3-54/3-0.03306235651)*2^20
const CBRT_P0: f64 = 1.87595182427177009643;
const CBRT_P1: f64 = -1.88497979543377169875;
const CBRT_P2: f64 = 1.621429720105354466140;
const CBRT_P3: f64 = -0.758397934778766047437;
const CBRT_P4: f64 = 0.145996192886612446982;

/// `cbrt`：fdlibm s_cbrt 忠实移植。
/// 旧实现（exp(log/3) + 3 次牛顿 + ±4 位邻域比较）实测 2 ulp：
/// 邻域比较用的是 `z*z*z - ax` 的 double 值，含两次舍入，选不出正确的那一位。
pub fn core_cbrt(x: f64) -> f64 {
    let bits = x.to_bits();
    let mut hx = (bits >> 32) as u32;
    let low = bits as u32;
    let sign = hx & 0x8000_0000;
    hx ^= sign;
    if hx >= 0x7ff0_0000 { return x + x; } // cbrt(NaN,Inf) 原样
    let mut t: f64;
    if hx < 0x0010_0000 {
        // 0 或次正规
        if (hx | low) == 0 { return x; }
        let s = f64::from_bits(0x4350_0000_0000_0000) * x; // ×2^54
        let high = get_high_word(s);
        t = set_high_word(0.0, sign | ((high & 0x7fff_ffff) / 3 + CBRT_B2));
    } else {
        t = set_high_word(0.0, sign | (hx / 3 + CBRT_B1));
    }
    // 粗糙初值（约 5 bit）→ 23 bit
    let r = (t * t) * (t / x);
    t = t * ((CBRT_P0 + r * (CBRT_P1 + r * CBRT_P2)) + ((r * r) * r) * (CBRT_P3 + r * CBRT_P4));
    // 向远离 0 的方向舍入到 23 位（保证 t 略大于真值）
    t = f64::from_bits((t.to_bits().wrapping_add(0x8000_0000)) & 0xffff_ffff_c000_0000);
    // 一次牛顿到 53 位
    let s = t * t;          // 精确（t 只有 23 位有效）
    let r = x / s;          // 误差 ≤ 0.5 ulp，|r| < |t|
    let w = t + t;          // 精确
    let r = (r - t) / (w + r); // r-t 精确；w+r ≈ 3t
    t + t * r
}

/// `hypot`：fdlibm e_hypot.c 忠实移植（文档保证误差 < 1 ulp）。
/// 旧实现（r=lo/hi → sqrt(1+r²) → ±1 位补偿）实测 2 ulp：补偿判据 `s²-t` 含两次舍入，
/// 而且 `hi*s` 的乘积舍入没有被补偿掉。
pub fn core_hypot(x: f64, y: f64) -> f64 {
    let mut ha = (get_high_word(x) & 0x7fff_ffff) as i32;
    let mut hb = (get_high_word(y) & 0x7fff_ffff) as i32;
    let (mut a, mut b) = if hb > ha { (y, x) } else { (x, y) };
    if hb > ha { let j = ha; ha = hb; hb = j; }
    a = core_fabs(a);
    b = core_fabs(b);
    if ha - hb > 0x3c0_0000 { return a + b; } // x/y > 2^60
    let mut k = 0i32;
    if ha > 0x5f30_0000 {
        // a > 2^500
        if ha >= 0x7ff0_0000 {
            let mut w = core_fabs(x + 0.0) - core_fabs(y + 0.0);
            let low = get_low_word(a);
            if ((ha as u32 & 0xfffff) | low) == 0 { w = a; }
            let low = get_low_word(b);
            if ((hb as u32 ^ 0x7ff0_0000) | low) == 0 { w = b; }
            return w;
        }
        ha -= 0x2580_0000; hb -= 0x2580_0000; k += 600;
        a = set_high_word(a, ha as u32);
        b = set_high_word(b, hb as u32);
    }
    if hb < 0x20b0_0000 {
        // b < 2^-500
        if hb <= 0x000f_ffff {
            let low = get_low_word(b);
            if ((hb as u32) | low) == 0 { return a; }
            let t1 = set_high_word(0.0, 0x7fd0_0000); // 2^1022
            b *= t1;
            a *= t1;
            k -= 1022;
        } else {
            ha += 0x2580_0000;
            hb += 0x2580_0000;
            k -= 600;
            a = set_high_word(a, ha as u32);
            b = set_high_word(b, hb as u32);
        }
    }
    let mut w = a - b;
    if w > b {
        let t1 = set_high_word(0.0, ha as u32);
        let t2 = a - t1;
        w = core_sqrt(t1 * t1 - (b * (-b) - t2 * (a + t1)));
    } else {
        a = a + a;
        let y1 = set_high_word(0.0, hb as u32);
        let y2 = b - y1;
        let t1 = set_high_word(0.0, (ha + 0x0010_0000) as u32);
        let t2 = a - t1;
        w = core_sqrt(t1 * y1 - (w * (-w) - (t1 * y2 + t2 * b)));
    }
    if k != 0 {
        let t1 = set_high_word(1.0, get_high_word(1.0).wrapping_add((k as u32) << 20));
        t1 * w
    } else {
        w
    }
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
