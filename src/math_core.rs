// 数学函数**纯计算核心**（无 syscall、无 crate 依赖）——为的是能在**宿主上用真实 libm 对照验证**。
//
// ## 精度目标（B 档，≤1 ulp）与验证方式
//
// 每项实现后由 `tools/checks/math_verify/verify.rs` 在宿主上跑：
// 用 `include!` 引入本文件，与宿主 glibc 的 libm 在**大样本 + 边界值**上逐点比对，
// 输出**实测最大 ULP 误差**。**未通过的不接进 libc**。
//
// ## 为什么单独成文件（不直接写在 libc 里）
//
// 若把 `#[no_mangle] extern "C"` 的导出与本文件放一起，宿主验证程序会与 glibc 的 libm
// 产生**重复符号**而链不上。故：**核心在此（纯函数）**，导出在 `math_exports.rs`。

/// `fabs`：清符号位（含 -0 → +0）。
#[inline]
pub fn core_fabs(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & 0x7fff_ffff_ffff_ffff)
}

/// `copysign`：把 y 的符号位给 x（含 ±0）。
#[inline]
pub fn core_copysign(x: f64, y: f64) -> f64 {
    f64::from_bits((x.to_bits() & 0x7fff_ffff_ffff_ffff) | (y.to_bits() & 0x8000_0000_0000_0000))
}

/// `floor`：向 -inf 取整。
pub fn core_floor(x: f64) -> f64 {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if exp >= 52 { return x; }           // 已是整数或 inf/nan
    if exp < 0 {
        // |x| < 1。**±0 必须原样返回（保号）**——验证器实测：首版在此把 floor(-0.0) 返回成 -1.0。
        if x == 0.0 {
            return x;
        }
        return if bits & 0x8000_0000_0000_0000 != 0 { -1.0 } else { 0.0 };
    }
    let mask = (1u64 << (52 - exp)) - 1;
    if bits & mask == 0 { return x; }
    let truncated = f64::from_bits(bits & !mask);
    if bits & 0x8000_0000_0000_0000 != 0 { truncated - 1.0 } else { truncated }
}

/// `ceil`：向 +inf 取整。
pub fn core_ceil(x: f64) -> f64 {
    -core_floor(-x)
}

/// `trunc`：向 0 取整。
pub fn core_trunc(x: f64) -> f64 {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if exp >= 52 { return x; }
    if exp < 0 { return f64::from_bits(bits & 0x8000_0000_0000_0000); } // ±0
    let mask = (1u64 << (52 - exp)) - 1;
    f64::from_bits(bits & !mask)
}

/// `round`：四舍五入到最近整数，**halfway 远离 0**（C 语义，与 `rint` 的 half-even 不同）。
pub fn core_round(x: f64) -> f64 {
    let t = core_trunc(x);
    let d = core_fabs(x - t);
    if d < 0.5 {
        t
    } else if d > 0.5 {
        t + core_copysign(1.0, x)
    } else {
        // 恰为 .5：远离 0（若 t 为偶/奇无关，C 的 round 一律远离 0）
        t + core_copysign(1.0, x)
    }
}

/// `modf`：拆分整数与小数部分（返回小数部分，整数写入 `*iptr`）。**符号跟随 x**。
pub fn core_modf(x: f64, iptr: &mut f64) -> f64 {
    if !x.is_finite() {
        *iptr = x;
        return if x.is_nan() { x } else { core_copysign(0.0, x) };
    }
    let t = core_trunc(x);
    *iptr = t;
    if core_fabs(x) < 1.0 {
        // |x|<1：整数部分为 ±0，小数部分为 x（保号）
        *iptr = core_copysign(0.0, x);
        return x;
    }
    x - t
}

/// `fmod`：C 语义的浮点余数（结果符号跟随 x）。用精确的整数化路径（对常见量级无误差）。
pub fn core_fmod(x: f64, y: f64) -> f64 {
    if y == 0.0 || x.is_nan() || y.is_nan() || x.is_infinite() {
        return f64::NAN;
    }
    if y.is_infinite() { return x; }
    if x == 0.0 { return x; }
    let ax = core_fabs(x);
    let ay = core_fabs(y);
    if ax < ay { return x; }
    // 逐位减法（二进制长除法），结果精确：fmod 要求精确。
    let mut r = ax;
    // 把 ay 左移到与 ax 同量级
    let mut d = ay;
    while d * 2.0 <= r { d *= 2.0; }
    while r >= ay {
        if r >= d { r -= d; }
        d *= 0.5;
        if d < ay { break; }
    }
    core_copysign(r, x)
}

/// `sqrt`：**整数算法**，正确舍入（≤0.5 ulp）。
///
/// 旧实现（牛顿 + ±1 位邻域比较）实测 1 ulp：邻域比较用 `z*z - x` 的 double 值判断，
/// 而 `z*z`、`y*y` 各自带一次舍入，判据本身就含 1 ulp 噪声；而 asin/hypot/cbrt
/// 都把 sqrt 放在最后一环，1 ulp 会被放大成 2 ulp。
///
/// 现改为：把 x 写成 `M·2^E`（M 为 53 位整数，E = 无偏指数-52），
///   E 偶：S = round(√(M·2^52))，结果 = S·2^(E/2-26)
///   E 奇：S = round(√(M·2^53))，结果 = S·2^((E-1)/2-26)
/// 用 u128 整数二分开方得到 S（约 53 位，余数 > S 即进位），再按位拼回 f64。
/// S 与指数都是精确整数运算 ⇒ 结果是**正确舍入**的平方根。
fn isqrt_u128(n: u128) -> u128 {
    let mut lo: u128 = 0;
    let mut hi: u128 = 1u128 << 53;
    while lo < hi {
        let mid = (lo + hi + 1) >> 1;
        if mid * mid <= n { lo = mid } else { hi = mid - 1; }
    }
    lo
}

pub fn core_sqrt(x: f64) -> f64 {
    if x.is_nan() { return f64::NAN; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 || x.is_infinite() { return x; }
    let bits = x.to_bits();
    if bits & 0x7ff0_0000_0000_0000 == 0 {
        // 次正规数：先乘 2^54 变正规，开方后乘 2^-27（都是 2 的幂 ⇒ 精确，且不产生次正规结果）
        return core_sqrt(x * f64::from_bits(0x4350_0000_0000_0000))
            * f64::from_bits(0x3e40_0000_0000_0000);
    }
    let e = ((bits >> 52) & 0x7ff) as i64 - 1023 - 52;
    let m = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
    let (n, shift) = if e.rem_euclid(2) == 0 {
        ((m as u128) << 52, e / 2 - 26)
    } else {
        ((m as u128) << 53, (e - 1) / 2 - 26)
    };
    let s0 = isqrt_u128(n);
    let mut s = s0;
    if n - s0 * s0 > s0 { s += 1; }        // 最近舍入（无平局：中点 s+0.5 不可表示）
    let mut sh = shift;
    if s == (1u128 << 53) { s = 1u128 << 52; sh += 1; }
    let e_biased = (1075 + sh) as u64;
    f64::from_bits((e_biased << 52) | ((s as u64) & 0x000f_ffff_ffff_ffff))
}
