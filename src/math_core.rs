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

/// `sqrt`：牛顿迭代（初值用位技巧，3 次迭代后达到 ≤1 ulp；再做一次舍入修正）。
pub fn core_sqrt(x: f64) -> f64 {
    if x.is_nan() { return f64::NAN; }
    if x < 0.0 { return f64::NAN; }
    if x == 0.0 || x.is_infinite() { return x; }
    // 初值：位技巧 y ≈ 2^(e/2) * 1.m 的近似（经典 0x5fe6eb50c7b537a9 逆平方根法简化）
    let mut y = f64::from_bits((x.to_bits() >> 1) + 0x1ff8_0000_0000_0000);
    // 牛顿迭代 y = (y + x/y)/2，5 次足够收敛到 ulp 级
    for _ in 0..5 {
        y = 0.5 * (y + x / y);
    }
    // 舍入修正：确保返回最接近真值的可表示数
    let y2 = y * y;
    if y2 > x {
        let z = f64::from_bits(y.to_bits() - 1);
        if core_fabs(z * z - x) < core_fabs(y2 - x) { z } else { y }
    } else if y2 < x {
        let z = f64::from_bits(y.to_bits() + 1);
        if core_fabs(z * z - x) < core_fabs(y2 - x) { z } else { y }
    } else {
        y
    }
}
