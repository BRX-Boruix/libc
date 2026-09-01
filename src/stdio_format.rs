//! printf 格式引擎（纯逻辑，可移植，可在 host 上单测）。
//!
//! 本模块实现 C printf 格式串的解析与渲染核心，不依赖任何系统调用，输出到
//! 一个可注入的字节 sink（\`FmtSink\`）。真正的 \`printf\`/\`sprintf\`/\`fprintf\`
//! 入口在 \`stdio.rs\`，它们经 \`c_variadic\` 读取可变参数后调用本引擎。
//!
//! 支持的转换：
//! - 整数：\`%d %i %u %x %X %o %b\`（含长度修饰 hh/h/l/ll/z，flags \`-+ 0#\`）
//! - 字符/字符串/指针：\`%c %s %p\`
//! - 浮点：\`%f %F %e %E %g %G\`（f64，经 c_variadic 读入）
//! - 其他：\`%%\`、\`%n\`（写已输出字符数）

/// 输出 sink 抽象（写字节；失败返回 Err 以中止格式化）。
use alloc::vec::Vec;

pub trait FmtSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()>;
    fn write_byte(&mut self, b: u8) -> Result<(), ()> {
        self.write(core::slice::from_ref(&b))
    }
}

/// 把输出写到一个 \`&mut Vec<u8>\` 的适配（sprintf/snprintf 用，可单测）。
pub struct VecSink<'a>(pub &'a mut Vec<u8>);
impl FmtSink for VecSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

// ---------- 格式说明符的解析结果 ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Length {
    None,
    Hh,
    H,
    L,
    Ll,
    Z,
    T,
    BigL,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Conv {
    Int,
    UInt,
    Oct,
    Hex,
    Bin,
    Char,
    Str,
    Ptr,
    Float,
    Exp,
    General,
    Percent,
    Count,
}

#[derive(Clone, Copy)]
pub struct Spec {
    pub left: bool,
    pub plus: bool,
    pub space: bool,
    pub zero: bool,
    pub alt: bool,
    pub width: i64,
    pub prec: i64,
    pub len: Length,
    pub conv: Conv,
    pub upper: bool,
}

/// 解析一段格式串，对每条说明调用 \`f\`。遇到 \`%%\` 作为字面量输出。
pub fn parse_and_format<F: FnMut(&Spec, &mut dyn FmtSink) -> Result<(), ()>>(
    fmt: &[u8],
    sink: &mut dyn FmtSink,
    mut f: F,
) -> Result<usize, ()> {
    let mut i = 0;
    let mut emitted = 0usize;
    while i < fmt.len() {
        let b = fmt[i];
        if b == b'%' {
            match parse_spec(fmt, i + 1) {
                Some((spec, next)) => {
                    if spec.conv == Conv::Percent {
                        sink.write_byte(b'%')?;
                        emitted += 1;
                    } else {
                        f(&spec, sink)?;
                        emitted += 1;
                    }
                    i = next;
                    continue;
                }
                None => {
                    sink.write_byte(b'%')?;
                    emitted += 1;
                    i += 1;
                    continue;
                }
            }
        }
        sink.write_byte(b)?;
        emitted += 1;
        i += 1;
    }
    Ok(emitted)
}

/// 尝试从 \`fmt[pos..]\` 解析一个格式说明符。
fn parse_spec(fmt: &[u8], mut pos: usize) -> Option<(Spec, usize)> {
    let mut spec = Spec {
        left: false, plus: false, space: false, zero: false, alt: false,
        width: -1, prec: -1, len: Length::None, conv: Conv::Int, upper: false,
    };
    loop {
        let c = *fmt.get(pos)?;
        match c {
            b'-' => spec.left = true,
            b'+' => spec.plus = true,
            b' ' => spec.space = true,
            b'0' => spec.zero = true,
            b'#' => spec.alt = true,
            _ => break,
        }
        pos += 1;
    }
    if *fmt.get(pos)? == b'*' {
        spec.width = -2;
        pos += 1;
    } else {
        let start = pos;
        while fmt.get(pos).map_or(false, |c| c.is_ascii_digit()) {
            pos += 1;
        }
        if pos > start {
            spec.width = parse_num(&fmt[start..pos])?;
        }
    }
    if *fmt.get(pos)? == b'.' {
        pos += 1;
        if *fmt.get(pos)? == b'*' {
            spec.prec = -2;
            pos += 1;
        } else {
            let start = pos;
            while fmt.get(pos).map_or(false, |c| c.is_ascii_digit()) {
                pos += 1;
            }
            if pos > start {
                spec.prec = parse_num(&fmt[start..pos])?;
            } else {
                spec.prec = 0;
            }
        }
    }
    match *fmt.get(pos)? {
        b'h' => {
            pos += 1;
            if *fmt.get(pos)? == b'h' {
                spec.len = Length::Hh;
                pos += 1;
            } else {
                spec.len = Length::H;
            }
        }
        b'l' => {
            pos += 1;
            if *fmt.get(pos)? == b'l' {
                spec.len = Length::Ll;
                pos += 1;
            } else {
                spec.len = Length::L;
            }
        }
        b'z' => { spec.len = Length::Z; pos += 1; }
        b't' => { spec.len = Length::T; pos += 1; }
        b'L' => { spec.len = Length::BigL; pos += 1; }
        b'j' => { spec.len = Length::Ll; pos += 1; }
        _ => {}
    }
    let c = *fmt.get(pos)?;
    spec.conv = match c {
        b'd' | b'i' => Conv::Int,
        b'u' => Conv::UInt,
        b'o' => Conv::Oct,
        b'x' => { spec.upper = false; Conv::Hex }
        b'X' => { spec.upper = true; Conv::Hex }
        b'b' => Conv::Bin,
        b'c' => Conv::Char,
        b's' => Conv::Str,
        b'p' => Conv::Ptr,
        b'f' => { spec.upper = false; Conv::Float }
        b'F' => { spec.upper = true; Conv::Float }
        b'e' => { spec.upper = false; Conv::Exp }
        b'E' => { spec.upper = true; Conv::Exp }
        b'g' => { spec.upper = false; Conv::General }
        b'G' => { spec.upper = true; Conv::General }
        b'%' => Conv::Percent,
        b'n' => Conv::Count,
        _ => return None,
    };
    Some((spec, pos + 1))
}

fn parse_num(digits: &[u8]) -> Option<i64> {
    let mut v: i64 = 0;
    for &d in digits {
        if !d.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((d - b'0') as i64)?;
    }
    Some(v)
}

/// 按 length 把原始 u64 截断/符号扩展成有效位宽。
pub fn fit_unsigned(raw: u64, len: Length, signed: bool) -> u64 {
    let bits: u64 = match len {
        Length::Hh => 8,
        Length::H => 16,
        Length::None | Length::L => 32,
        Length::Ll | Length::Z | Length::T | Length::BigL => 64,
    };
    if bits == 64 {
        return raw;
    }
    let mask = (1u64 << bits) - 1;
    let val = raw & mask;
    if signed {
        let sign_bit = 1u64 << (bits - 1);
        if val & sign_bit != 0 {
            return val | (!mask);
        }
    }
    val
}

fn write_digits(v: u64, base: u32, upper: bool, buf: &mut [u8]) -> usize {
    if base == 10 {
        return write_dec(v, buf);
    }
    const DIGITS_LOWER: &[u8; 16] = b"0123456789abcdef";
    const DIGITS_UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let digits: &[u8] = if upper { DIGITS_UPPER } else { DIGITS_LOWER };
    let mut i = buf.len();
    let mut v = v;
    loop {
        i -= 1;
        buf[i] = digits[(v % base as u64) as usize];
        v /= base as u64;
        if v == 0 {
            break;
        }
    }
    buf.len() - i
}

fn write_dec(mut v: u64, buf: &mut [u8]) -> usize {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    buf.len() - i
}

/// 渲染一个有符号/无符号整数说明符到 sink。
pub fn emit_int(
    spec: &Spec,
    value: u64,
    is_negative: bool,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    let (base, prefix): (u32, &[u8]) = match spec.conv {
        Conv::Int | Conv::UInt => (10, b""),
        Conv::Oct => (8, if spec.alt && value != 0 { b"0" } else { b"" }),
        Conv::Hex => (
            16,
            if spec.alt && value != 0 {
                if spec.upper { b"0X" } else { b"0x" }
            } else {
                b""
            },
        ),
        Conv::Bin => (2, if spec.alt && value != 0 { b"0b" } else { b"" }),
        _ => (10, b""),
    };
    let mut digits_buf = [0u8; 64];
    let ndigits = write_digits(value, base, spec.upper, &mut digits_buf);

    let min_digits = if spec.prec >= 0 { spec.prec as usize } else { 1 };
    let pad_zeros = min_digits.saturating_sub(ndigits);

    let sign: &[u8] = if is_negative { b"-" } else if spec.plus { b"+" } else if spec.space { b" " } else { b"" };

    let content_len = sign.len() + prefix.len() + pad_zeros + ndigits;
    let width = spec.width.max(0) as usize;
    let pad_total = width.saturating_sub(content_len);
    let pad_char: u8 = if spec.zero && !spec.left && spec.prec < 0 { b'0' } else { b' ' };

    if spec.left {
        sink.write(sign)?;
        sink.write(prefix)?;
        for _ in 0..pad_zeros { sink.write_byte(b'0')?; }
        sink.write(&digits_buf[digits_buf.len() - ndigits..])?;
        for _ in 0..pad_total { sink.write_byte(b' ')?; }
    } else if pad_char == b'0' {
        sink.write(sign)?;
        sink.write(prefix)?;
        for _ in 0..pad_total { sink.write_byte(b'0')?; }
        for _ in 0..pad_zeros { sink.write_byte(b'0')?; }
        sink.write(&digits_buf[digits_buf.len() - ndigits..])?;
    } else {
        for _ in 0..pad_total { sink.write_byte(b' ')?; }
        sink.write(sign)?;
        sink.write(prefix)?;
        for _ in 0..pad_zeros { sink.write_byte(b'0')?; }
        sink.write(&digits_buf[digits_buf.len() - ndigits..])?;
    }
    Ok(())
}

/// 渲染一个字符串说明符到 sink。
pub fn emit_str(spec: &Spec, s: &[u8], sink: &mut dyn FmtSink) -> Result<(), ()> {
    let len = if spec.prec >= 0 {
        (spec.prec as usize).min(s.len())
    } else {
        s.len()
    };
    let width = spec.width.max(0) as usize;
    let pad = width.saturating_sub(len);
    if spec.left {
        sink.write(&s[..len])?;
        for _ in 0..pad { sink.write_byte(b' ')?; }
    } else {
        for _ in 0..pad { sink.write_byte(b' ')?; }
        sink.write(&s[..len])?;
    }
    Ok(())
}

pub fn emit_char(spec: &Spec, c: u8, sink: &mut dyn FmtSink) -> Result<(), ()> {
    emit_str(spec, core::slice::from_ref(&c), sink)
}
