//! 大整数精确 dtoa 分解（float.rs 的 decompose 核心替换）。
//!
//! 思路（Dragon4 精确法）：value = m × 2^e2（m 为 53 位尾数整数，e2 为合并
//! 指数）。把 value 写成 `N × 10^-k`（N 为大整数）：
//! - e2 >= 0：N = m << e2，k = 0
//! - e2 <  0：k = -e2，N = m × 5^k（因 value = m×5^k / 10^k）
//! 则 N 的十进制展开即 value 的精确数字，小数点从 N 右侧数 k 位。
//! dec_exp（emit_* 约定，整数部分位数 = dec_exp+1）= digit_count(N) - k - 1。
//!
//! 覆盖全部 f64 值域（含 2^1074 分母的次正规数），逐位精确。

/// 大整数（无符号，little-endian u32 肢）。f64 最坏情况约 800 十进制位
/// （次正规 5e-324），对应约 2670 bit ≈ 84 肢；200 肢留足除法中间量余量。
const MAX_LIMBS: usize = 220;

/// 大整数（无符号，little-endian u32 肢，len 表示有效肢数）。
pub struct BigInt {
    pub limbs: [u32; MAX_LIMBS],
    pub len: usize,
}

impl BigInt {
    pub fn zero() -> Self {
        BigInt { limbs: [0; MAX_LIMBS], len: 0 }
    }
    pub fn from_u64(v: u64) -> Self {
        let mut b = BigInt::zero();
        if v == 0 {
            return b;
        }
        b.limbs[0] = v as u32;
        if v > 0xFFFF_FFFF {
            b.limbs[1] = (v >> 32) as u32;
            b.len = 2;
        } else {
            b.len = 1;
        }
        b
    }
    pub fn is_zero(&self) -> bool {
        self.len == 0
    }
    /// 乘小整数（< 2^32）。
    pub fn mul_small(&mut self, m: u32) {
        if self.is_zero() {
            return;
        }
        let mut carry: u64 = 0;
        let mut i = 0usize;
        while i < self.len {
            let cur = self.limbs[i] as u64 * m as u64 + carry;
            self.limbs[i] = cur as u32;
            carry = cur >> 32;
            i += 1;
        }
        if carry != 0 {
            if self.len >= MAX_LIMBS {
                return;
            }
            self.limbs[self.len] = carry as u32;
            self.len += 1;
        }
    }
    /// 加小整数（< 2^32）。
    pub fn add_small(&mut self, d: u32) {
        if d == 0 {
            return;
        }
        if self.is_zero() {
            self.limbs[0] = d;
            self.len = 1;
            return;
        }
        let mut carry: u64 = d as u64;
        let mut i = 0usize;
        while carry != 0 && i < MAX_LIMBS {
            let cur = self.limbs[i] as u64 + carry;
            self.limbs[i] = cur as u32;
            carry = cur >> 32;
            i += 1;
        }
        if i > self.len {
            self.len = i;
        }
    }

    /// 乘 u64（拆成两个 32 位乘法，先低位后高位）。
    pub fn mul_u64(&mut self, m: u64) {
        let lo = m as u32;
        let hi = (m >> 32) as u32;
        // 先乘 lo（低位），再乘 hi 并左移 32 位累加。
        let mut a = BigInt { limbs: self.limbs, len: self.len };
        a.mul_small(lo);
        let mut b = BigInt { limbs: self.limbs, len: self.len };
        b.mul_small(hi);
        b.shl(32);
        self.copy_add(&a, &b);
    }
    fn copy_add(&mut self, a: &BigInt, b: &BigInt) {
        let max = a.len.max(b.len);
        let mut carry: u64 = 0;
        let mut i = 0usize;
        while i < max {
            let av = if i < a.len { a.limbs[i] as u64 } else { 0 };
            let bv = if i < b.len { b.limbs[i] as u64 } else { 0 };
            let sum = av + bv + carry;
            self.limbs[i] = sum as u32;
            carry = sum >> 32;
            i += 1;
        }
        if carry != 0 {
            self.limbs[max] = carry as u32;
            self.len = max + 1;
        } else {
            self.len = max;
        }
        while self.len > 0 && self.limbs[self.len - 1] == 0 {
            self.len -= 1;
        }
    }
    /// 左移 sh 位。
    pub fn shl(&mut self, sh: u32) {
        if self.is_zero() || sh == 0 {
            return;
        }
        let limb_shift = (sh / 32) as usize;
        let bit_shift = sh % 32;
        let old_len = self.len;
        if old_len + limb_shift + 1 > MAX_LIMBS {
            return;
        }
        // 从高到低搬移（仅当跨肢；limb_shift=0 时搬移会清零数据）。
        if limb_shift > 0 {
            let mut i = old_len;
            while i > 0 {
                i -= 1;
                self.limbs[i + limb_shift] = self.limbs[i];
                self.limbs[i] = 0;
            }
        }
        if bit_shift != 0 {
            let mut carry: u32 = 0;
            let mut j = 0usize;
            while j < old_len + limb_shift + 1 {
                let cur = self.limbs[j];
                self.limbs[j] = (cur << bit_shift) | carry;
                carry = cur >> (32 - bit_shift);
                j += 1;
            }
        }
        self.len = old_len + limb_shift + 1;
        while self.len > 0 && self.limbs[self.len - 1] == 0 {
            self.len -= 1;
        }
    }
    /// 除以小整数，商就地更新，返回余数。
    pub fn div_small(&mut self, d: u32) -> u32 {
        let mut rem: u64 = 0;
        let mut i = self.len;
        while i > 0 {
            i -= 1;
            let cur = (rem << 32) | self.limbs[i] as u64;
            self.limbs[i] = (cur / d as u64) as u32;
            rem = cur % d as u64;
        }
        while self.len > 0 && self.limbs[self.len - 1] == 0 {
            self.len -= 1;
        }
        rem as u32
    }

    /// 逐位长除法：self /= d，商就地更新，返回余数（self >= 0, d > 0）。
    /// 使用二进制恢复除法（位级），O(bits^2)，对 strtod 的 53 位尾数提取足够。
    pub fn divrem(&mut self, d: &BigInt) -> BigInt {
        let mut rem = BigInt::zero();
        let mut q = BigInt::zero();
        let bl = self.bit_len();
        let mut i = bl;
        while i > 0 {
            i -= 1;
            // rem = rem << 1 | bit(i)
            rem.shl(1);
            if self.get_bit(i) {
                if rem.limbs.len() > 0 && rem.len < MAX_LIMBS {
                    rem.limbs[0] |= 1;
                    if rem.len == 0 { rem.len = 1; }
                }
            }
            if rem.cmp(d) != core::cmp::Ordering::Less {
                rem.sub_assign(d);
                q.set_bit(i);
            }
        }
        *self = q;
        rem
    }

    /// 比较：self 与 other。
    pub fn cmp(&self, other: &BigInt) -> core::cmp::Ordering {
        if self.len != other.len {
            return self.len.cmp(&other.len);
        }
        let mut i = self.len;
        while i > 0 {
            i -= 1;
            if self.limbs[i] != other.limbs[i] {
                return self.limbs[i].cmp(&other.limbs[i]);
            }
        }
        core::cmp::Ordering::Equal
    }

    /// 就地减：self -= other（要求 self >= other）。
    pub fn sub_assign(&mut self, other: &BigInt) {
        let mut borrow: u64 = 0;
        let mut i = 0usize;
        let max = self.len.max(other.len);
        while i < max {
            let a = if i < self.len { self.limbs[i] as u64 } else { 0 };
            let b = if i < other.len { other.limbs[i] as u64 } else { 0 };
            let (r, bo) = a.overflowing_sub(b + borrow);
            self.limbs[i] = r as u32;
            borrow = if bo { 1 } else { 0 };
            i += 1;
        }
        while self.len > 0 && self.limbs[self.len - 1] == 0 {
            self.len -= 1;
        }
    }

    /// 读第 i 位（0 = LSB）。
    pub fn get_bit(&self, i: u32) -> bool {
        let limb = (i / 32) as usize;
        if limb >= self.len { return false; }
        (self.limbs[limb] >> (i % 32)) & 1 == 1
    }

    /// 置第 i 位为 1。
    pub fn set_bit(&mut self, i: u32) {
        let limb = (i / 32) as usize;
        if limb >= MAX_LIMBS { return; }
        if limb >= self.len { self.len = limb + 1; }
        self.limbs[limb] |= 1 << (i % 32);
    }

    /// 有效位数（最高置位 + 1；0 返回 0）。
    pub fn bit_len(&self) -> u32 {
        if self.len == 0 { return 0; }
        let top = self.limbs[self.len - 1];
        (32 * (self.len as u32)) - top.leading_zeros()
    }

}

/// 计算 5^k（k 非负，小端 u32 肢）。
fn pow5(k: u32) -> BigInt {
    let mut b = BigInt::from_u64(1);
    let mut kk = k;
    let base: u32 = 5;
    while kk > 0 {
        b.mul_small(base);
        kk -= 1;
    }
    b
}

/// 大整数转十进制数字（升序：least significant 在前）。返回数字个数 L。
/// 通过反复除以 10 取余得到。
fn bigint_to_digits(b: &BigInt, out: &mut [u8]) -> usize {
    let mut w = BigInt { limbs: b.limbs, len: b.len };
    let mut n = 0usize;
    loop {
        let r = w.div_small(10);
        if n < out.len() {
            out[n] = r as u8 + b'0';
            n += 1;
        }
        if w.is_zero() {
            break;
        }
        if n >= out.len() {
            break;
        }
    }
    if n == 0 {
        out[0] = b'0';
        n = 1;
    }
    n
}

/// 分解 f64 为 `0.digits × 10^dec_exp`（精确，全值域）。
pub fn decompose_exact(value: f64, digits: &mut [u8; 800]) -> (usize, i32, bool, u8) {
    let bits = value.to_bits();
    let sign = (bits >> 63) != 0;
    let biased_exp = ((bits >> 52) & 0x7FF) as i64;
    let mant = bits & ((1u64 << 52) - 1);

    // 特殊值。
    if biased_exp == 0x7FF {
        if mant == 0 {
            return (0, 0, sign, if sign { 2 } else { 1 });
        } else {
            return (0, 0, sign, 3);
        }
    }

    // 尾数 m（53 位，含隐式位；次正规无隐式位）。
    let m: u64 = if biased_exp == 0 {
        mant
    } else {
        mant | (1u64 << 52)
    };
    if m == 0 {
        // 零。
        digits[0] = b'0';
        return (1, 0, sign, 0);
    }
    let e2 = if biased_exp == 0 {
        -1074
    } else {
        (biased_exp - 1023) - 52
    };

    // 把 value 写成 N × 10^-k。
    let (k, n): (u32, BigInt) = if e2 >= 0 {
        let mut n = BigInt::from_u64(m);
        n.shl(e2 as u32);
        (0, n)
    } else {
        let k = (-e2) as u32;
        let mut n = pow5(k);
        n.mul_u64(m);
        (k, n)
    };

    // N 的十进制数字（升序 least-first）。
    let mut dseq = [0u8; 800];
    let l = bigint_to_digits(&n, &mut dseq);
    // 反转成 most-first 到 digits。
    let mut idx = 0usize;
    while idx < l {
        digits[idx] = dseq[l - 1 - idx];
        idx += 1;
    }
    // dec_exp（emit_* 约定：整数部分位数 = dec_exp+1，即 floor(log10))。
    // value = N × 10^-k，N 有 l 位，最高位在 10^(l-1-k)；故 dec_exp = l-1-k。
    let dec_exp = l as i64 - k as i64 - 1;
    (l, dec_exp as i32, sign, 0)
}

/// 把正大整数 m（低 64 位足够）的最低 53 位取出（忽略更高位；调用方保证 ≤ 53 位）。
fn low_53(m: &BigInt) -> u64 {
    let mut lo: u64 = m.limbs[0] as u64;
    if m.len >= 2 { lo |= (m.limbs[1] as u64) << 32; }
    lo & ((1u64 << 53) - 1)
}

/// 用 53 位尾数 m 与指数 exp2 组装 f64（value = m × 2^exp2，m ∈ [2^52,2^53) 或次正规后 m < 2^52）。
/// 处理次正规与溢出（溢出返回 inf）。
fn build_double(m: u64, exp2: i64) -> f64 {
    // 正常数：exp2 ∈ [-1074, 971] 时 m ∈ [2^52,2^53)，value = (2^52+f)×2^(e-1075)。
    if exp2 >= -1074 {
        if exp2 > 971 {
            return f64::INFINITY; // 溢出
        }
        let biased = (exp2 + 1075) as u64; // e 域
        let f = m - (1u64 << 52);       // 尾数域（m 应为 53 位）
        f64::from_bits(biased << 52 | f)
    } else {
        // 次正规：exp2 < -1074，value = m×2^exp2，须右移为 2^-1074 的倍数。
        let shift = (-1074 - exp2) as u64;
        if shift >= 63 {
            // m（≤2^53）右移 ≥63 位 → 0 或舍入到 1（最小次正规）。
            // 仅当 m 的最高位接近 2^shift 时可能进位到 1。
            // shift ≥ 63 > 53，m/2^shift 商为 0（m<2^53），仅舍入到 0（不可能到 1）。
            return 0.0;
        }
        let dropped = m & ((1u64 << shift) - 1);
        let mut m2 = m >> shift;
        let half = 1u64 << (shift - 1);
        if dropped > half || (dropped == half && (m2 & 1) == 1) {
            m2 += 1;
        }
        if m2 >= (1u64 << 52) {
            // 进位到最小正常数 2^-1022。
            f64::from_bits(1u64 << 52)
        } else {
            // 次正规：直接尾数域（偏置指数 0）。
            f64::from_bits(m2)
        }
    }
}


/// 舍入判定：rem/den 的小数部分是否 > 0.5（Greater）、== 0.5（Equal）或 < 0.5（Less）。
#[inline]
fn frac_vs_half(rem: &BigInt, den: &BigInt) -> core::cmp::Ordering {
    let mut two_rem = BigInt { limbs: rem.limbs, len: rem.len };
    two_rem.shl(1);
    two_rem.cmp(den)
}

/// 把正分数 (num/den)×2^shift 正确舍入为 f64（round-half-even，含次正规边界）。
/// num/den 为正，shift 可为负。溢出返回 inf。
fn rational_to_f64(num: &BigInt, den: &BigInt, shift: i64) -> f64 {
    if num.is_zero() {
        return 0.0;
    }
    let rbl = num.bit_len() as i64;
    let sbl = den.bit_len() as i64;
    // 估算 e = floor(log2(value)) - 52 ≈ shift + (rbl-1) - (sbl-1) - 52。
    let mut e = shift + (rbl - sbl) - 52;
    // 微调 e，使 q = floor(value / 2^e) ∈ [2^52, 2^53)。
    let (mut q, mut rem, mut den_used): (BigInt, BigInt, BigInt);
    loop {
        let (num_n, den_n): (BigInt, BigInt) = if e <= shift {
            // value/2^e = num/den × 2^(shift-e)，shift-e >= 0 → 分子左移。
            let mut n = BigInt { limbs: num.limbs, len: num.len };
            n.shl((shift - e) as u32);
            (n, BigInt { limbs: den.limbs, len: den.len })
        } else {
            let mut d = BigInt { limbs: den.limbs, len: den.len };
            d.shl((e - shift) as u32);
            (BigInt { limbs: num.limbs, len: num.len }, d)
        };
        q = BigInt { limbs: num_n.limbs, len: num_n.len };
        rem = q.divrem(&den_n);
        den_used = den_n;
        let qbl = q.bit_len() as i64;
        if qbl > 53 {
            e += 1;
            continue;
        }
        if qbl < 53 {
            e -= 1;
            continue;
        }
        break;
    }
    // 正常数：e >= -1074。
    if e >= -1074 {
        if e > 971 {
            return f64::INFINITY;
        }
        let mut m = low_53(&q);
        let ord = frac_vs_half(&rem, &den_used);
        if ord == core::cmp::Ordering::Greater || (ord == core::cmp::Ordering::Equal && (m & 1) == 1) {
            m += 1;
            if m == (1u64 << 53) {
                // 进位：m=2^53 → 2^52，e+1。
                return build_double(1u64 << 52, e + 1);
            }
        }
        build_double(m, e)
    } else {
        // 次正规：e < -1074。value = (num/den)×2^shift，需折叠到 exp2=-1074。
        // 次正规：value = (num/den)×2^shift，折叠到 exp2=-1074。
        // s = value×2^1074 = num/(den×2^(-1074-shift))。
        let sh = (-1074 - shift) as i64;
        let (num_s, den_s): (BigInt, BigInt) = if sh >= 0 {
            let mut d = BigInt { limbs: den.limbs, len: den.len };
            d.shl(sh as u32);
            (BigInt { limbs: num.limbs, len: num.len }, d)
        } else {
            let mut n = BigInt { limbs: num.limbs, len: num.len };
            n.shl((-sh) as u32);
            (n, BigInt { limbs: den.limbs, len: den.len })
        };
        let mut s = BigInt { limbs: num_s.limbs, len: num_s.len };
        let rem_s = s.divrem(&den_s);
        let ord = frac_vs_half(&rem_s, &den_s);
        let mut m2 = low_53(&s);
        // 次正规尾数在 [1, 2^52)；若进位到 2^52 → 最小正常数。
        if ord == core::cmp::Ordering::Greater || (ord == core::cmp::Ordering::Equal && (m2 & 1) == 1) {
            m2 += 1;
        }
        if m2 >= (1u64 << 52) {
            // 进位到最小正常数 2^-1022。
            f64::from_bits(1u64 << 52)
        } else {
            // 次正规：直接尾数域（偏置指数 0）。
            f64::from_bits(m2)
        }
    }
}

/// 精确正确舍入：value = mant × 10^(exp - frac_digits)，舍入到最近 f64（tie 到偶）。
///
/// mant 为去掉小数点的全部有效数字（BigInt），frac_digits 为小数点后的数字数，
/// exp 为显式指数部分。与 dtoa（decompose_exact）互补：这是字符串→f64 的精确路径。
/// 返回正确舍入的 f64（0 返回 0.0，溢出返回 ±inf）。
pub fn strtod_exact(mant: &BigInt, frac_digits: i64, exp: i64) -> f64 {
    if mant.is_zero() {
        return 0.0;
    }
    let k = frac_digits - exp; // value = mant × 10^-k
    if k <= 0 {
        // value = mant × 5^(-k) × 2^(-k) = (mant×5^(-k))×2^(-k)。
        let mut num = BigInt { limbs: mant.limbs, len: mant.len };
        let p5 = (-k) as u32;
        for _ in 0..p5 {
            num.mul_small(5);
        }
        let den = BigInt::from_u64(1);
        return rational_to_f64(&num, &den, -k);
    }
    // k > 0：value = mant / 10^k = mant / (5^k × 2^k)。
    let mut den = BigInt::from_u64(1);
    for _ in 0..k {
        den.mul_small(5);
    }
    den.shl(k as u32); // 再 ×2^k → 10^k
    let num = BigInt { limbs: mant.limbs, len: mant.len };
    rational_to_f64(&num, &den, 0)
}

/// 取正大整数 m 的最低 24 位（f32 尾数）。
fn low_24(m: &BigInt) -> u32 {
    m.limbs[0] & ((1u32 << 24) - 1)
}

/// 用 24 位尾数 m 与指数 exp2 组装 f32（value = m × 2^exp2，m ∈ [2^23,2^24) 或次正规后 m < 2^23）。
/// 处理次正规与溢出（溢出返回 inf）。
fn build_float(m: u32, exp2: i64) -> f32 {
    if exp2 >= -149 {
        if exp2 > 104 {
            return f32::INFINITY;
        }
        let biased = (exp2 + 150) as u32; // e 域：127 + 23 位尾数
        let f = m - (1u32 << 23);
        f32::from_bits(biased << 23 | f)
    } else {
        let shift = (-149 - exp2) as u32;
        if shift >= 24 {
            return 0.0;
        }
        let dropped = m & ((1u32 << shift) - 1);
        let mut m2 = m >> shift;
        let half = 1u32 << (shift - 1);
        if dropped > half || (dropped == half && (m2 & 1) == 1) {
            m2 += 1;
        }
        if m2 >= (1u32 << 23) {
            f32::from_bits(1u32 << 23)
        } else {
            f32::from_bits(m2)
        }
    }
}

/// 把正分数 (num/den)×2^shift 正确舍入为 f32（round-half-even，含次正规边界）。
fn rational_to_f32(num: &BigInt, den: &BigInt, shift: i64) -> f32 {
    if num.is_zero() {
        return 0.0;
    }
    let rbl = num.bit_len() as i64;
    let sbl = den.bit_len() as i64;
    let mut e = shift + (rbl - sbl) - 23;
    let (mut q, mut rem, mut den_used): (BigInt, BigInt, BigInt);
    loop {
        let (num_n, den_n): (BigInt, BigInt) = if e <= shift {
            let mut n = BigInt { limbs: num.limbs, len: num.len };
            n.shl((shift - e) as u32);
            (n, BigInt { limbs: den.limbs, len: den.len })
        } else {
            let mut d = BigInt { limbs: den.limbs, len: den.len };
            d.shl((e - shift) as u32);
            (BigInt { limbs: num.limbs, len: num.len }, d)
        };
        q = BigInt { limbs: num_n.limbs, len: num_n.len };
        rem = q.divrem(&den_n);
        den_used = den_n;
        let qbl = q.bit_len() as i64;
        if qbl > 24 {
            e += 1;
            continue;
        }
        if qbl < 24 {
            e -= 1;
            continue;
        }
        break;
    }
    if e >= -149 {
        if e > 104 {
            return f32::INFINITY;
        }
        let mut m = low_24(&q);
        let ord = frac_vs_half(&rem, &den_used);
        if ord == core::cmp::Ordering::Greater || (ord == core::cmp::Ordering::Equal && (m & 1) == 1) {
            m += 1;
            if m == (1u32 << 24) {
                return build_float(1u32 << 23, e + 1);
            }
        }
        build_float(m, e)
    } else {
        let sh = (-149 - shift) as i64;
        let (num_s, den_s): (BigInt, BigInt) = if sh >= 0 {
            let mut d = BigInt { limbs: den.limbs, len: den.len };
            d.shl(sh as u32);
            (BigInt { limbs: num.limbs, len: num.len }, d)
        } else {
            let mut n = BigInt { limbs: num.limbs, len: num.len };
            n.shl((-sh) as u32);
            (n, BigInt { limbs: den.limbs, len: den.len })
        };
        let mut s = BigInt { limbs: num_s.limbs, len: num_s.len };
        let rem_s = s.divrem(&den_s);
        let ord = frac_vs_half(&rem_s, &den_s);
        let mut m2 = low_24(&s);
        if ord == core::cmp::Ordering::Greater || (ord == core::cmp::Ordering::Equal && (m2 & 1) == 1) {
            m2 += 1;
        }
        if m2 >= (1u32 << 23) {
            f32::from_bits(1u32 << 23)
        } else {
            f32::from_bits(m2)
        }
    }
}

/// 精确正确舍入：value = mant × 10^(exp - frac_digits)，舍入到最近 f32（tie 到偶）。
/// 返回正确舍入的 f32（0 返回 0.0，溢出返回 ±inf）。
pub fn strtof_exact(mant: &BigInt, frac_digits: i64, exp: i64) -> f32 {
    if mant.is_zero() {
        return 0.0;
    }
    let k = frac_digits - exp; // value = mant × 10^-k
    if k <= 0 {
        let mut num = BigInt { limbs: mant.limbs, len: mant.len };
        let p5 = (-k) as u32;
        for _ in 0..p5 {
            num.mul_small(5);
        }
        let den = BigInt::from_u64(1);
        return rational_to_f32(&num, &den, -k);
    }
    let mut den = BigInt::from_u64(1);
    for _ in 0..k {
        den.mul_small(5);
    }
    den.shl(k as u32);
    let num = BigInt { limbs: mant.limbs, len: mant.len };
    rational_to_f32(&num, &den, 0)
}




