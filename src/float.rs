//! f64 十进制格式化（`%f/%e/%g` 的 dtoa 核心）与 `%a` 十六进制浮点。
//!
//! 纯逻辑、可移植、可 host 单测。把 f64 分解为 \`0.digits × 10^dec_exp\` 形式，
//! \`%f/%e/%g\` 据此重排与舍入。
//!
//! ## 精度 / 范围保证（S33/S29 如实声明）
//!
//! - \`precision <= 17\`（默认 6）时，对常规 f64 输入给出与 C 库一致或仅末位
//!   相差 1 ulp 的十进制串；这是 f64 十进制 round-trip 的有效位上限。
//! - 采用 round-half-even 舍入（C 默认方向）。
//! - **全范围精确**：本实现用**大整数精确法**（float_bigint.rs）把 f64 视为
//!   \`m × 2^e2\` 并精确展开为十进制，覆盖全部值域（含最大/最小次正规数
//!   如 \`1e300\` 与 \`5e-324\`），逐位精确，无窗口限制。
//! - 性能为朴素大数除法（每位一次全量除法），printf 默认精度足够；吞吐优化
//!   （S32 无优化无数据）留待需要时量测。
//! - 特殊值 inf/-inf/nan 显式输出（S09，绝不伪造数值）。

use crate::stdio_format::{Spec, FmtSink};

/// 浮点值的分解结果（未舍入的原始数字序列，最多 800 位）。
#[derive(Clone)]
pub struct Decomposed {
    /// 数字位（\`0..=9\` 的 ASCII），不含小数点。
    pub digits: [u8; 800],
    pub digit_len: usize,
    /// 十进制指数：数值 = 0.digits × 10^dec_exp。
    pub dec_exp: i32,
    pub negative: bool,
    /// 特殊值：1=+inf 2=-inf 3=nan。
    pub special: u8,
}

/// 分解 f64，不做舍入（输出可精确生成的十进制数字，**全值域精确**）。
///
/// 经大整数精确法（\`decompose_exact\`，float_bigint.rs）把 value 写成
/// \`N × 10^-k\` 并展开 N 的十进制数字，覆盖最大/最小次正规数（S33 如实）。
pub fn decompose(value: f64) -> Decomposed {
    // 快速路径：小整数直接展开（避免大整数除法）；否则回退精确路径。
    if let Some(d) = decompose_fast(value) {
        return d;
    }
    let mut digits = [0u8; 800];
    let (len, dec_exp, neg, special) = crate::float_bigint::decompose_exact(value, &mut digits);
    Decomposed {
        digits,
        digit_len: len,
        dec_exp,
        negative: neg,
        special,
    }
}

/// `%f/%e/%g` 的快速路径：当 value 是**可精确表示的小整数**（|value| <= 2^63）
/// 时，直接十进制展开，避免大整数除法。返回 Some(展开结果) 或 None（回退精确路径）。
/// 判定：把 value 拆成 m × 2^e2（e2>=0）且整数部分 m<<e2 不溢出 i64 范围。
pub fn decompose_fast(value: f64) -> Option<Decomposed> {
    let bits = value.to_bits();
    let sign = (bits >> 63) != 0;
    let biased_exp = ((bits >> 52) & 0x7FF) as i64;
    let mant = bits & ((1u64 << 52) - 1);

    if biased_exp == 0x7FF || (mant == 0 && biased_exp == 0) {
        return None;
    }
    let m: u64 = if biased_exp == 0 { mant } else { mant | (1u64 << 52) };
    let e2 = if biased_exp == 0 { -1074 } else { (biased_exp - 1023) - 52 };

    // value = m × 2^e2。若 e2<0 但 m 含足够尾随零位，值仍是整数（如 2^51）。
    if e2 < 0 {
        let shift = (-e2) as u32;
        if shift >= 64 || m & ((1u64 << shift) - 1) != 0 {
            return None; // 非整数或无法归约。
        }
        let m2 = m >> shift;
        if m2 > (1u64 << 63) { return None; }
        let mut digits = [0u8; 800];
        let mut buf = [0u8; 20];
        let mut v = m2;
        let mut i = buf.len();
        loop {
            i -= 1;
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 { break; }
        }
        let l = buf.len() - i;
        let mut idx = 0usize;
        while idx < l { digits[idx] = buf[i + idx]; idx += 1; }
        return Some(Decomposed { digits, digit_len: l, dec_exp: l as i32 - 1, negative: sign, special: 0 });
    }
    if e2 > 62 { return None; }
    let int_val: u64 = m << e2;
    if int_val > (1u64 << 63) { return None; }
    let mut digits = [0u8; 800];
    let mut buf = [0u8; 20];
    let mut v = int_val;
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 { break; }
    }
    let l = buf.len() - i;
    let mut idx = 0usize;
    while idx < l {
        digits[idx] = buf[i + idx];
        idx += 1;
    }
    Some(Decomposed {
        digits,
        digit_len: l,
        dec_exp: l as i32 - 1,
        negative: sign,
        special: 0,
    })
}

/// 舍入到 \`n\` 个有效数字（round-half-even），就地更新 digit_len。
/// 若进位溢出最高位，dec_exp +1 且 digit_len=1（"1"）。
pub fn round_to(d: &mut Decomposed, n: usize) {
    if n >= d.digit_len {
        return;
    }
    let keep = n;
    let mut round_up = false;
    if keep < d.digit_len {
        let next = d.digits[keep];
        if next > b'5' {
            round_up = true;
        } else if next == b'5' {
            let mut any = false;
            for i in (keep + 1)..d.digit_len {
                if d.digits[i] != b'0' {
                    any = true;
                    break;
                }
            }
            round_up = any || (keep > 0 && (d.digits[keep - 1] - b'0') % 2 == 1);
        }
    }
    d.digit_len = keep;
    if round_up {
        let mut i = keep as isize - 1;
        loop {
            if i < 0 {
                d.digits[0] = b'1';
                d.digit_len = 1;
                d.dec_exp += 1;
                return;
            }
            if d.digits[i as usize] == b'9' {
                d.digits[i as usize] = b'0';
                i -= 1;
            } else {
                d.digits[i as usize] += 1;
                return;
            }
        }
    }
}
// ---------- 输出辅助（栈上缓冲） ----------

/// 小型栈上字节缓冲（避免分配）。
pub struct StackBuf<const N: usize> {
    pub buf: [u8; N],
    pub len: usize,
}
impl<const N: usize> StackBuf<N> {
    pub fn new() -> Self { StackBuf { buf: [0; N], len: 0 } }
    pub fn push_byte(&mut self, b: u8) {
        if self.len < N {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }
    pub fn extend(&mut self, s: &[u8]) {
        for &b in s {
            self.push_byte(b);
        }
    }
    pub fn as_slice(&self) -> &[u8] { &self.buf[..self.len] }
    pub fn pop_byte(&mut self) -> Option<u8> {
        if self.len == 0 { None } else { self.len -= 1; Some(self.buf[self.len]) }
    }
    pub fn is_empty(&self) -> bool { self.len == 0 }
}

/// 写一个字节数组，带 width/left/zero 对齐。
fn emit_aligned(
    spec: &Spec,
    out: &[u8],
    sign_len: usize,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    let width = spec.width.max(0) as usize;
    let pad = width.saturating_sub(out.len());
    if spec.left {
        sink.write(out)?;
        for _ in 0..pad { sink.write_byte(b' ')?; }
    } else if spec.zero && pad > 0 {
        sink.write(&out[..sign_len])?;
        for _ in 0..pad { sink.write_byte(b'0')?; }
        sink.write(&out[sign_len..])?;
    } else {
        for _ in 0..pad { sink.write_byte(b' ')?; }
        sink.write(out)?;
    }
    Ok(())
}

fn sign_prefix(d: &Decomposed, spec: &Spec) -> &'static [u8] {
    if d.negative { b"-" } else if spec.plus { b"+" } else if spec.space { b" " } else { b"" }
}

/// \`%e/%E\`：\`d.ddde±XX\`（precision 为小数位数）。
pub fn emit_exp(
    spec: &Spec,
    d: &mut Decomposed,
    precision: usize,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    if d.special != 0 {
        return emit_special(spec, d, sink);
    }
    round_to(d, precision + 1);
    let sign = sign_prefix(d, spec);
    let mut body = StackBuf::<420>::new();
    body.extend(sign);
    body.push_byte(d.digits[0]);
    if precision > 0 {
        body.push_byte(b'.');
        for i in 1..=precision {
            let c = if i < d.digit_len { d.digits[i] } else { b'0' };
            body.push_byte(c);
        }
    }
    let marker = if spec.upper { b'E' } else { b'e' };
    body.push_byte(marker);
    // 语义：value = d0.d1d2... × 10^dec_exp。%e 需 d.ddd × 10^XX，XX = dec_exp。
    let e = d.dec_exp;
    let (e_neg, e_abs) = if e < 0 { (true, (-e) as u32) } else { (false, e as u32) };
    let mut ebuf = [0u8; 8];
    let mut ei = ebuf.len();
    let mut v = e_abs;
    loop {
        ei -= 1;
        ebuf[ei] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 { break; }
    }
    let e_digits = ebuf.len() - ei;
    if e_neg { body.push_byte(b'-'); } else { body.push_byte(b'+'); }
    if e_digits < 2 { body.push_byte(b'0'); }
    body.extend(&ebuf[ei..]);
    emit_aligned(spec, body.as_slice(), sign.len(), sink)
}

/// \`%f/%F\`：定点小数（precision 为小数位数）。
pub fn emit_fixed(
    spec: &Spec,
    d: &mut Decomposed,
    precision: usize,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    if d.special != 0 {
        return emit_special(spec, d, sink);
    }
    let int_pos = d.dec_exp + 1;
    let need = if int_pos >= 0 {
        int_pos as usize + precision
    } else {
        (-int_pos) as usize + precision
    };
    round_to(d, need.max(1));

    let sign = sign_prefix(d, spec);
    let int_pos = d.dec_exp + 1;
    let num_digits = d.digit_len as i64;
    let mut body = StackBuf::<420>::new();
    body.extend(sign);

    let int_len = int_pos.max(0) as usize;
    if int_len == 0 {
        body.push_byte(b'0');
    } else {
        for i in 0..int_len {
            let c = if (i as i64) < num_digits { d.digits[i] } else { b'0' };
            body.push_byte(c);
        }
    }
    if precision > 0 {
        body.push_byte(b'.');
        if int_pos >= 0 {
            // 小数位从数字序列索引 int_pos 开始，共 precision 位。
            for k in 0..precision as i64 {
                let idx = int_pos as i64 + k;
                let c = if idx >= 0 && idx < num_digits { d.digits[idx as usize] } else { b'0' };
                body.push_byte(c);
            }
        } else {
            // 值 < 0.1：先补 -int_pos 个前导 0（这些计入 precision），
            // 再从 digits[0] 起补足 precision 位。
            let lead = (-int_pos) as usize;
            for k in 0..precision {
                if k < lead {
                    body.push_byte(b'0');
                } else {
                    let idx = (k - lead) as i64;
                    let c = if idx < num_digits { d.digits[idx as usize] } else { b'0' };
                    body.push_byte(c);
                }
            }
        }
    }
    emit_aligned(spec, body.as_slice(), sign.len(), sink)
}

/// \`%g/%G\`：自动选择 %e 或 %f。
pub fn emit_general(
    spec: &Spec,
    d: &mut Decomposed,
    mut precision: usize,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    if precision == 0 {
        precision = 1;
    }
    if d.special != 0 {
        return emit_special(spec, d, sink);
    }
    round_to(d, precision);
    let x = d.dec_exp;
    if x < -4 || x >= precision as i32 {
        emit_exp(spec, d, precision - 1, sink)
    } else {
        emit_fixed_g(spec, d, precision, sink)
    }
}

/// %g 的 %f 变体：去掉尾随 0，整数位非零则去掉小数点。
fn emit_fixed_g(
    spec: &Spec,
    d: &mut Decomposed,
    precision: usize,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    let sign = sign_prefix(d, spec);
    let int_pos = d.dec_exp + 1;
    let num_digits = d.digit_len as i64;
    let int_len = int_pos.max(0) as usize;
    let mut body = StackBuf::<420>::new();
    body.extend(sign);
    if int_len == 0 {
        body.push_byte(b'0');
    } else {
        for i in 0..int_len {
            let c = if (i as i64) < num_digits { d.digits[i] } else { b'0' };
            body.push_byte(c);
        }
    }
    let mut last_nonzero: i64 = -1;
    if int_pos >= 0 {
        for k in 0..precision as i64 {
            let idx = int_pos as i64 + k;
            let c = if idx >= 0 && idx < num_digits { d.digits[idx as usize] } else { b'0' };
            if c != b'0' { last_nonzero = k; }
        }
    } else {
        let lead = (-int_pos) as i64;
        for k in 0..precision as i64 {
            let c = if k < lead {
                b'0'
            } else {
                let idx = k - lead;
                if idx < num_digits { d.digits[idx as usize] } else { b'0' }
            };
            if c != b'0' { last_nonzero = k; }
        }
    }
    if last_nonzero >= 0 {
        body.push_byte(b'.');
        if int_pos >= 0 {
            for k in 0..=last_nonzero {
                let idx = int_pos as i64 + k;
                let c = if idx >= 0 && idx < num_digits { d.digits[idx as usize] } else { b'0' };
                body.push_byte(c);
            }
        } else {
            let lead = (-int_pos) as usize;
            for k in 0..=(last_nonzero as usize) {
                if k < lead {
                    body.push_byte(b'0');
                } else {
                    let idx = (k - lead) as i64;
                    let c = if idx < num_digits { d.digits[idx as usize] } else { b'0' };
                    body.push_byte(c);
                }
            }
        }
    }
    emit_aligned(spec, body.as_slice(), sign.len(), sink)
}

/// 特殊值输出：inf / nan。


/// `%a/%A`（十六进制浮点）：把 f64 输出为 `0xh.hhhhp±d`。
/// 十六进制表示是**精确**的（无需舍入）：正常数形如 0x1.xxx×2^e，
/// 次正规数形如 0x0.xxx×2^-1022。默认精度为足够精确表示全部 13 位十六进制
/// 尾数数字；`#` 强制显示小数点与尾随零；大写 `%A` 用 `X/P`。
/// 特殊值 inf/nan 与 `%f` 相同（经 decompose 的 special 标记）。
pub fn emit_hexfloat(spec: &Spec, v: f64, sink: &mut dyn FmtSink) -> Result<(), ()> {
    let bits = v.to_bits();
    let sign = (bits >> 63) & 1 == 1;
    let exp_field = ((bits >> 52) & 0x7FF) as i64;
    let frac = bits & 0x000F_FFFF_FFFF_FFFF;
    // sign_len：零填充时保留的前导符号字节数。
    let sign_len = if sign { 1 } else if spec.plus || spec.space { 1 } else { 0 };

    // 特殊值：inf/nan。
    if exp_field == 0x7FF {
        let word: &[u8] = if frac != 0 {
            if spec.upper { b"NAN" } else { b"nan" }
        } else if sign {
            b"-inf"
        } else {
            b"inf"
        };
        // 加号/空格标志。
        let mut s = StackBuf::<32>::new();
        if word == b"-inf" {
            s.extend(word);
        } else {
            if spec.plus { s.push_byte(b'+'); }
            else if spec.space { s.push_byte(b' '); }
            s.extend(word);
        }
        return emit_aligned(spec, s.as_slice(), sign_len, sink)
    }

    // 零。
    if bits == 0 || bits == 0x8000_0000_0000_0000 {
        let precision = if spec.prec >= 0 { spec.prec as usize } else { 0 };
        let mut s = StackBuf::<64>::new();
        if sign { s.push_byte(b'-'); }
        else if spec.plus { s.push_byte(b'+'); }
        else if spec.space { s.push_byte(b' '); }
        s.extend(if spec.upper { b"0X0" } else { b"0x0" });
        if precision > 0 || spec.alt {
            s.push_byte(b'.');
            for _ in 0..precision { s.push_byte(b'0'); }
        }
        s.push_byte(b'p');
        s.push_byte(b'+');
        s.extend(b"0");
        return emit_aligned(spec, s.as_slice(), sign_len, sink)
    }

    let upper = spec.upper;
    let digits: &[u8; 16] = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    let mut s = StackBuf::<80>::new();
    if sign { s.push_byte(b'-'); }
    else if spec.plus { s.push_byte(b'+'); }
    else if spec.space { s.push_byte(b' '); }
    s.extend(if upper { b"0X" } else { b"0x" });

    // 正常数：0x1.xxxp+e；次正规：0x0.xxxp-1022。
    let (first, exp2) = if exp_field == 0 {
        // 次正规：值 = 0.frac × 2^-1022。
        (0u64, -1022i64)
    } else {
        (1u64, exp_field - 1023)
    };

    // 默认精度 = 需表示 frac 的最低非零位所需的十六进制数字数（尾随零截断）。
    // 正常数隐含前导 1；次正规前导为 0。
    let lowest_set = if frac == 0 {
        0 // 无小数位。
    } else {
        // frac 最低非零位下标（0..=51），换算为十六进制数字数（每 4 位一个）。
        let lsb = frac.trailing_zeros() as u32; // 0..52
        // 从最高位组到该组共需的 hex 数字：52 位 → 13 组，最后一组 4 位对齐。
        // 最低非零位所在组：lsb/4（0 基）。它之后到最高组共有 13 - lsb/4 组？
        // 精确：有效 hex 位数 = ceil((51 - lsb + 1)/4)？用更直观方式：
        // frac 有效位宽 = 52 - lsb；hex 数字数 = ceil(有效位宽/4)。
        let width = 52 - lsb;
        width.div_ceil(4)
    };
    let default_prec = lowest_set as usize;
    let precision = if spec.prec >= 0 { spec.prec as usize } else { default_prec };

    // 首数字。
    s.push_byte(digits[first as usize]);
    // 逐 4 位输出 frac 的十六进制数字到临时缓冲。
    let n_hex: usize = 13; // 52 位 → 13 个 hex 数字。
    let mut frac_buf = StackBuf::<16>::new();
    for i in 0..n_hex {
        if frac_buf.len >= precision {
            break;
        }
        let shift = 52 - 4 - (i as u32) * 4; // 最高组 i=0: shift=48。
        let nib = ((frac >> shift) & 0xF) as usize;
        frac_buf.push_byte(digits[nib]);
    }
    // 显式精度下补足。
    while spec.prec >= 0 && frac_buf.len < precision {
        frac_buf.push_byte(b'0');
    }
    // 有小数数字或 # 标志 → 显示小数点（0x1.xxx）；否则省略（0x1p+0）。
    if frac_buf.len > 0 || spec.alt {
        s.push_byte(b'.');
        s.extend(frac_buf.as_slice());
    }
    s.push_byte(if upper { b'P' } else { b'p' });
    if exp2 >= 0 { s.push_byte(b'+'); } else { s.push_byte(b'-'); }
    let mut ebuf = [0u8; 24];
    let mut v = exp2.abs();
    let mut ei = ebuf.len();
    loop {
        ei -= 1;
        ebuf[ei] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 { break; }
    }
    s.extend(&ebuf[ei..]);
    emit_aligned(spec, s.as_slice(), sign_len, sink)
}


fn emit_special(spec: &Spec, d: &Decomposed, sink: &mut dyn FmtSink) -> Result<(), ()> {
    let word: &[u8] = match d.special {
        1 => b"inf",
        2 => b"-inf",
        3 => {
            if spec.upper { b"NAN" } else { b"nan" }
        }
        _ => b"",
    };
    let width = spec.width.max(0) as usize;
    let pad = width.saturating_sub(word.len());
    if spec.left {
        sink.write(word)?;
        for _ in 0..pad { sink.write_byte(b' ')?; }
    } else {
        for _ in 0..pad { sink.write_byte(b' ')?; }
        sink.write(word)?;
    }
    Ok(())
}
