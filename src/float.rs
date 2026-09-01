//! f64 十进制格式化（\`%f/%e/%g\` 的 dtoa 核心）。
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
