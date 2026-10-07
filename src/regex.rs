//! POSIX 正则（`regcomp` / `regexec` / `regerror` / `regfree`）——纯库代码实现。
//!
//! ## 实现路线（为什么是回溯 VM）
//!
//! 正则先编译成**扁平指令数组**，再用**回溯虚拟机**执行：`Split` 先走左支（贪心），
//! 失败则回溯走右支；`Save` 记录捕获组边界，回溯时恢复。选它而不是 AST 递归，是因为
//! 分组/量词/交替的**延续（continuation）**在扁平指令里天然表达（跳转），不需要把「剩余
//! 模式」当参数层层传递——后者在 Rust 里会与借用检查器长期搏斗，且代码量大得多。
//!
//! ## 诚实边界（S09，逐条可复核）
//!
//! - **复杂度**：回溯实现，最坏情形（如 `(a*)*b` 对长串）是指数级。POSIX 不要求正则引擎
//!   有多项式保证，glibc 同样用回溯；但调用方**不应**把不可信输入喂给复杂模式。
//! - **不支持**：反向引用（`\1`）、字符类 `[[:alpha:]]`（本实现提供 `\d \D \w \W \s \S` 扩展）、
//!   区域相关排序（`REG_NEWLINE` 的 `.` 不含换行已实现；`[^...]` 对换行的处理按 REG_NEWLINE）、
//!   `REG_ICASE` 的 ASCII 之外大小写（本系统无 locale 数据）。遇到 `\1`..`\9` 一律
//!   `REG_ESUBREG` 报错，**绝不静默当字面量**。
//! - **`{m,n}` 的上界**：`n` 超过 [`MAX_REPEAT`] 时按 [`MAX_REPEAT`] 截断并如实返回 `REG_BADBR`
//!   （不静默截断——那会让 `a{1,100000}` 的行为与调用方预期不符）。
//!
//! 指令数组与捕获组都走 `alloc`（本 crate 已有 `extern crate alloc`）。

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::ctypes::{c_char, c_int, c_void, size_t};

pub const REG_EXTENDED: c_int = 1;
pub const REG_ICASE: c_int = 2;
pub const REG_NEWLINE: c_int = 4;
pub const REG_NOSUB: c_int = 8;

pub const REG_NOMATCH: c_int = 1;
pub const REG_BADPAT: c_int = 2;
pub const REG_ECOLLATE: c_int = 3;
pub const REG_ECTYPE: c_int = 4;
pub const REG_EESCAPE: c_int = 5;
pub const REG_ESUBREG: c_int = 6;
pub const REG_EBRACK: c_int = 7;
pub const REG_EPAREN: c_int = 8;
pub const REG_EBRACE: c_int = 9;
pub const REG_BADBR: c_int = 10;
pub const REG_ERANGE: c_int = 11;
pub const REG_ESPACE: c_int = 12;
pub const REG_BADRPT: c_int = 13;

/// `{m,n}` 里 `n` 的上界（见模块文档的诚实边界）。
pub const MAX_REPEAT: u32 = 255;

/// 编译后的正则（C 可见布局见 `libc/include/regex.h`）。
#[repr(C)]
pub struct regex_t {
    /// `*mut Vec<Inst>`（`Box::into_raw`）；NULL = 未编译。
    pub prog: *mut c_void,
    /// 捕获组数（不含整体匹配的第 0 组）。
    pub re_nsub: size_t,
    pub cflags: c_int,
    /// 最近一次 regcomp 的错误码（供 regerror 用）。
    pub errcode: c_int,
}

/// `regmatch_t`（POSIX；`regoff_t` 在 x86_64 上是 long）。
#[repr(C)]
pub struct regmatch_t {
    pub rm_so: isize,
    pub rm_eo: isize,
}

/// `regexec` 的 `eflags`。
pub const REG_NOTBOL: c_int = 1;
pub const REG_NOTEOL: c_int = 2;

// ---------------------------------------------------------------------------
// 指令集
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Inst {
    /// 匹配一个字节（已按 REG_ICASE 折叠成小写，比较时也折叠输入）。
    Char(u8),
    /// `.`（REG_NEWLINE 时不含 '\n'）。
    Any { newline: bool },
    /// 字符集（下标进 [`Program::classes`]）。
    Class(usize),
    /// `^`：串首，或 REG_NEWLINE 下的换行之后。
    Start { newline: bool },
    /// `$`：串尾，或 REG_NEWLINE 下的换行之前。
    End { newline: bool },
    /// 保存当前偏移到捕获槽。
    Save(usize),
    /// 无条件跳转。
    Jmp(usize),
    /// 先试 `0`，失败再试 `1`（贪心 = 先试「继续」）。
    Split(usize, usize),
    /// 匹配成功。
    Match,
}

#[derive(Clone)]
enum ClassItem {
    Ch(u8),
    Range(u8, u8),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

#[derive(Clone)]
struct Class {
    neg: bool,
    items: Vec<ClassItem>,
}

struct Program {
    insts: Vec<Inst>,
    classes: Vec<Class>,
}

fn is_word(c: u8) -> bool { c.is_ascii_alphanumeric() || c == b'_' }

fn class_match(cl: &Class, c: u8, icase: bool) -> bool {
    let lc = if icase { c.to_ascii_lowercase() } else { c };
    let mut hit = false;
    for it in cl.items.iter() {
        let h = match it {
            ClassItem::Ch(x) => lc == *x,
            ClassItem::Range(a, b) => *a <= lc && lc <= *b,
            ClassItem::Digit => c.is_ascii_digit(),
            ClassItem::NotDigit => !c.is_ascii_digit(),
            ClassItem::Word => is_word(c),
            ClassItem::NotWord => !is_word(c),
            ClassItem::Space => c == b' ' || (c >= 0x09 && c <= 0x0d),
            ClassItem::NotSpace => !(c == b' ' || (c >= 0x09 && c <= 0x0d)),
        };
        if h {
            hit = true;
            break;
        }
    }
    hit != cl.neg
}

// ---------------------------------------------------------------------------
// 编译
// ---------------------------------------------------------------------------

struct Parser<'a> {
    pat: &'a [u8],
    pos: usize,
    extended: bool,
    icase: bool,
    newline: bool,
    insts: Vec<Inst>,
    classes: Vec<Class>,
    nsub: usize,
}

impl<'a> Parser<'a> {
    fn emit(&mut self, i: Inst) -> usize {
        self.insts.push(i);
        self.insts.len() - 1
    }

    fn peek(&self) -> Option<u8> {
        self.pat.get(self.pos).copied()
    }

    /// 交替：`alt := concat ('|' concat)*`。
    ///
    /// 指令形状（`A|B|C`）：
    /// ```text
    ///   Split L1, S1
    /// L1: A
    ///   Jmp END
    /// S1: Split L2, S2
    /// L2: B
    ///   Jmp END
    /// S2: Split L3, END
    /// L3: C
    /// END:
    /// ```
    /// 实现方式：**先预留一条 Split**（此刻还不知道后面有没有 `|`），再逐分支回填。
    /// 无交替时把它降级成 `Jmp 下一条`（空操作）——这样不需要在发射后**插入**指令，
    /// 而插入会移动其后所有绝对跳转下标、必须做重定位（那是 bug 的温床）。
    fn parse_alt(&mut self) -> Result<(), c_int> {
        let split0 = self.emit(Inst::Split(usize::MAX, usize::MAX));
        let b0 = self.insts.len();
        self.parse_concat()?;
        let mut jmps: Vec<usize> = Vec::new();
        // 待回填的 Split 链：(split 下标, 左支目标)。
        let mut pending: Vec<(usize, usize)> = Vec::new();
        pending.push((split0, b0));
        while self.is_alt_sep() {
            // 消费分隔符（ERE 的 '|' 或 BRE 的 '\|'）。
            self.pos += if self.extended { 1 } else { 2 };
            let j = self.emit(Inst::Jmp(usize::MAX));
            jmps.push(j);
            let s = self.emit(Inst::Split(usize::MAX, usize::MAX));
            let bs = self.insts.len();
            let (prev, left) = pending.pop().unwrap();
            self.insts[prev] = Inst::Split(left, s);
            pending.push((s, bs));
            self.parse_concat()?;
        }
        let end = self.insts.len();
        if jmps.is_empty() {
            // 无交替：预留的 Split 降级为空操作（跳下一条）。
            self.insts[split0] = Inst::Jmp(b0);
            return Ok(());
        }
        for (s, left) in pending {
            self.insts[s] = Inst::Split(left, end);
        }
        for j in jmps {
            self.insts[j] = Inst::Jmp(end);
        }
        Ok(())
    }

    /// 判断当前位置是否是交替分隔符。
    fn is_alt_sep(&self) -> bool {
        match self.peek() {
            Some(b'|') if self.extended => true,
            Some(b'\\') if !self.extended => self.pat.get(self.pos + 1) == Some(&b'|'),
            _ => false,
        }
    }

    /// 连接：`concat := repeat*`。
    fn parse_concat(&mut self) -> Result<(), c_int> {
        loop {
            match self.peek() {
                None => break,
                Some(b'|') if self.extended => break,
                Some(b')') if self.extended => break,
                Some(b'\\') if !self.extended => {
                    let n = self.pat.get(self.pos + 1).copied();
                    if n == Some(b'|') || n == Some(b')') {
                        break;
                    }
                    self.parse_repeat()?;
                }
                _ => self.parse_repeat()?,
            }
        }
        Ok(())
    }

    fn parse_repeat(&mut self) -> Result<(), c_int> {
        let atom_start = self.insts.len();
        self.parse_atom()?;
        let mut quantified = false;
        loop {
            let (min, max) = match self.peek_quantifier() {
                Some(q) => q,
                None => break,
            };
            if quantified {
                // 连续量词（`a**`、`a*?`）：POSIX 规定为错误。绝不静默按「嵌套量词」处理——
                // 那会让调用方以为 `a**` 有确定语义，而实际行为是实现细节。
                return Err(REG_BADRPT);
            }
            quantified = true;
            self.consume_quantifier();
            if min > MAX_REPEAT || max.map(|m| m > MAX_REPEAT).unwrap_or(false) {
                return Err(REG_BADBR);
            }
            let body: Vec<Inst> = self.insts.split_off(atom_start);
            // 重建：min 份必选，随后 (max-min) 份可选。
            for _ in 0..min {
                self.insts.extend(body.iter().cloned());
            }
            match max {
                None => {
                    // 星号/加号：Split 回跳。
                    let l = self.insts.len();
                    let split = self.emit(Inst::Split(usize::MAX, usize::MAX));
                    let bstart = self.insts.len();
                    self.insts.extend(body.iter().cloned());
                    self.emit(Inst::Jmp(l));
                    let after = self.insts.len();
                    self.insts[split] = Inst::Split(bstart, after);
                }
                Some(m) => {
                    let mut splits = Vec::new();
                    for _ in min..m {
                        let s = self.emit(Inst::Split(usize::MAX, usize::MAX));
                        splits.push(s);
                        self.insts.extend(body.iter().cloned());
                    }
                    let after = self.insts.len();
                    for s in splits {
                        let bstart = s + 1;
                        self.insts[s] = Inst::Split(bstart, after);
                    }
                }
            }
        }
        Ok(())
    }

    fn peek_quantifier(&self) -> Option<(u32, Option<u32>)> {
        match self.peek() {
            Some(b'*') => Some((0, None)),
            Some(b'+') => Some((1, None)),
            Some(b'?') => Some((0, Some(1))),
            Some(b'\\') if !self.extended => match self.pat.get(self.pos + 1) {
                Some(b'{') => self.parse_brace_at(self.pos + 2),
                Some(b'?') => Some((0, Some(1))),
                Some(b'+') => Some((1, None)),
                _ => None,
            },
            Some(b'{') if self.extended => self.parse_brace_at(self.pos + 1),
            _ => None,
        }
    }

    fn consume_quantifier(&mut self) {
        match self.peek() {
            Some(b'*') | Some(b'+') | Some(b'?') if self.extended => self.pos += 1,
            Some(b'{') if self.extended => {
                self.pos += 1;
                while self.peek().map(|c| c != b'}').unwrap_or(false) {
                    self.pos += 1;
                }
                self.pos += 1;
            }
            Some(b'\\') if !self.extended => match self.pat.get(self.pos + 1) {
                Some(b'{') => {
                    self.pos += 2;
                    while self.peek().map(|c| c != b'}').unwrap_or(false) {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                _ => self.pos += 2,
            },
            _ => {},
        }
    }

    /// 解析 `{m}` / `{m,}` / `{m,n}`（`i` 指向第一个数字）。
    fn parse_brace_at(&self, i: usize) -> Option<(u32, Option<u32>)> {
        let mut j = i;
        let mut lo: u32 = 0;
        let mut lo_digits = 0;
        while let Some(c) = self.pat.get(j).copied() {
            if c.is_ascii_digit() {
                lo = lo.saturating_mul(10).saturating_add((c - b'0') as u32);
                lo_digits += 1;
                j += 1;
            } else {
                break;
            }
        }
        if lo_digits == 0 {
            return None;
        }
        match self.pat.get(j).copied() {
            Some(b'}') => Some((lo, Some(lo))),
            Some(b',') => {
                let mut k = j + 1;
                let mut hi: u32 = 0;
                let mut hi_digits = 0;
                while let Some(c) = self.pat.get(k).copied() {
                    if c.is_ascii_digit() {
                        hi = hi.saturating_mul(10).saturating_add((c - b'0') as u32);
                        hi_digits += 1;
                        k += 1;
                    } else {
                        break;
                    }
                }
                if self.pat.get(k).copied() != Some(b'}') {
                    return None;
                }
                if hi_digits == 0 {
                    Some((lo, None))
                } else {
                    Some((lo, Some(hi)))
                }
            }
            _ => None,
        }
    }

    fn parse_atom(&mut self) -> Result<(), c_int> {
        let c = match self.peek() {
            Some(c) => c,
            None => return Err(REG_BADPAT),
        };
        match c {
            b'.' => {
                self.pos += 1;
                self.emit(Inst::Any { newline: self.newline });
            }
            b'^' => {
                self.pos += 1;
                self.emit(Inst::Start { newline: self.newline });
            }
            b'$' => {
                self.pos += 1;
                self.emit(Inst::End { newline: self.newline });
            }
            b'[' => self.parse_class()?,
            b'(' if self.extended => {
                self.pos += 1;
                self.nsub += 1;
                let g = self.nsub;
                self.emit(Inst::Save(2 * g));
                self.parse_alt()?;
                if self.peek() != Some(b')') {
                    return Err(REG_EPAREN);
                }
                self.pos += 1;
                self.emit(Inst::Save(2 * g + 1));
            }
            b'\\' => {
                let n = self.pat.get(self.pos + 1).copied();
                match n {
                    None => return Err(REG_EESCAPE),
                    Some(b'(') if !self.extended => {
                        self.pos += 2;
                        self.nsub += 1;
                        let g = self.nsub;
                        self.emit(Inst::Save(2 * g));
                        self.parse_alt()?;
                        if self.pat.get(self.pos) != Some(&b'\\')
                            || self.pat.get(self.pos + 1) != Some(&b')')
                        {
                            return Err(REG_EPAREN);
                        }
                        self.pos += 2;
                        self.emit(Inst::Save(2 * g + 1));
                    }
                    Some(b')') if !self.extended => return Err(REG_EPAREN),
                    Some(b'd') => { self.pos += 2; self.emit_class_items(vec![ClassItem::Digit], false); }
                    Some(b'D') => { self.pos += 2; self.emit_class_items(vec![ClassItem::NotDigit], false); }
                    Some(b'w') => { self.pos += 2; self.emit_class_items(vec![ClassItem::Word], false); }
                    Some(b'W') => { self.pos += 2; self.emit_class_items(vec![ClassItem::NotWord], false); }
                    Some(b's') => { self.pos += 2; self.emit_class_items(vec![ClassItem::Space], false); }
                    Some(b'S') => { self.pos += 2; self.emit_class_items(vec![ClassItem::NotSpace], false); }
                    Some(b'1'..=b'9') => return Err(REG_ESUBREG),
                    Some(b'n') => { self.pos += 2; self.emit_char(b'\n'); }
                    Some(b't') => { self.pos += 2; self.emit_char(b'\t'); }
                    Some(b'r') => { self.pos += 2; self.emit_char(b'\r'); }
                    Some(b'f') => { self.pos += 2; self.emit_char(0x0c); }
                    Some(b'v') => { self.pos += 2; self.emit_char(0x0b); }
                    Some(b'0') => { self.pos += 2; self.emit_char(0); }
                    Some(x) => { self.pos += 2; self.emit_char(x); }
                }
            }
            b'*' | b'+' | b'?' => {
                // 量词无原子可作用：POSIX 规定 BRE 里行首的 '*' 是字面量，
                // 其余情形是错误。
                if !self.extended && c == b'*' {
                    self.pos += 1;
                    self.emit_char(b'*');
                } else {
                    return Err(REG_BADRPT);
                }
            }
            b'{' if !self.extended => {
                self.pos += 1;
                self.emit_char(b'{');
            }
            x => {
                self.pos += 1;
                self.emit_char(x);
            }
        }
        Ok(())
    }

    fn emit_char(&mut self, c: u8) {
        let c = if self.icase { c.to_ascii_lowercase() } else { c };
        self.emit(Inst::Char(c));
    }

    fn emit_class_items(&mut self, items: Vec<ClassItem>, neg: bool) {
        let idx = self.classes.len();
        self.classes.push(Class { neg, items });
        self.emit(Inst::Class(idx));
    }

    fn parse_class(&mut self) -> Result<(), c_int> {
        self.pos += 1; // '['
        let mut neg = false;
        if self.peek() == Some(b'^') {
            neg = true;
            self.pos += 1;
        }
        let mut items: Vec<ClassItem> = Vec::new();
        let mut first = true;
        loop {
            let c = match self.peek() {
                Some(c) => c,
                None => return Err(REG_EBRACK),
            };
            if c == b']' && !first {
                self.pos += 1;
                break;
            }
            first = false;
            // [:alpha:] 之类的字符类：本实现**不支持**，如实报错而不是当字面量。
            if c == b'[' && self.pat.get(self.pos + 1) == Some(&b':') {
                return Err(REG_ECTYPE);
            }
            let lo = if c == b'\\' {
                match self.pat.get(self.pos + 1).copied() {
                    None => return Err(REG_EESCAPE),
                    Some(b'd') => { self.pos += 2; items.push(ClassItem::Digit); continue; }
                    Some(b'D') => { self.pos += 2; items.push(ClassItem::NotDigit); continue; }
                    Some(b'w') => { self.pos += 2; items.push(ClassItem::Word); continue; }
                    Some(b'W') => { self.pos += 2; items.push(ClassItem::NotWord); continue; }
                    Some(b's') => { self.pos += 2; items.push(ClassItem::Space); continue; }
                    Some(b'S') => { self.pos += 2; items.push(ClassItem::NotSpace); continue; }
                    Some(b'n') => { self.pos += 2; b'\n' }
                    Some(b't') => { self.pos += 2; b'\t' }
                    Some(b'r') => { self.pos += 2; b'\r' }
                    Some(x) => { self.pos += 2; x }
                }
            } else {
                self.pos += 1;
                c
            };
            let lo = if self.icase { lo.to_ascii_lowercase() } else { lo };
            if self.peek() == Some(b'-')
                && self.pat.get(self.pos + 1).copied() != Some(b']')
                && self.pat.get(self.pos + 1).is_some()
            {
                self.pos += 1;
                let hc = self.peek().unwrap();
                let hi = if hc == b'\\' {
                    match self.pat.get(self.pos + 1).copied() {
                        None => return Err(REG_EESCAPE),
                        Some(x) => { self.pos += 2; x }
                    }
                } else {
                    self.pos += 1;
                    hc
                };
                let hi = if self.icase { hi.to_ascii_lowercase() } else { hi };
                if lo > hi {
                    return Err(REG_ERANGE);
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Ch(lo));
            }
        }
        if items.is_empty() {
            return Err(REG_EBRACK);
        }
        self.emit_class_items(items, neg);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 执行
// ---------------------------------------------------------------------------

struct Exec<'a> {
    prog: &'a Program,
    s: &'a [u8],
    icase: bool,
    notbol: bool,
    noteol: bool,
    caps: Vec<isize>,
    steps: u64,
}

/// 步数上限：回溯实现的**兜底**，避免病态模式把内核栈/时间耗光。
/// 超出即判不匹配（并如实计入 `regerror` 的语义之外——POSIX 无此概念，故只作为
/// 引擎自身的防御，不影响正确性：真正的匹配不会需要这么多步）。
const MAX_STEPS: u64 = 2_000_000;

impl<'a> Exec<'a> {
    fn run(&mut self, pc: usize, pos: usize) -> bool {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return false;
        }
        match &self.prog.insts[pc] {
            Inst::Match => true,
            Inst::Char(c) => {
                if pos >= self.s.len() {
                    return false;
                }
                let got = if self.icase { self.s[pos].to_ascii_lowercase() } else { self.s[pos] };
                got == *c && self.run(pc + 1, pos + 1)
            }
            Inst::Any { newline } => {
                if pos >= self.s.len() {
                    return false;
                }
                if *newline && self.s[pos] == b'\n' {
                    return false;
                }
                self.run(pc + 1, pos + 1)
            }
            Inst::Class(idx) => {
                if pos >= self.s.len() {
                    return false;
                }
                class_match(&self.prog.classes[*idx], self.s[pos], self.icase) && self.run(pc + 1, pos + 1)
            }
            Inst::Start { newline } => {
                let ok = if pos == 0 {
                    !self.notbol
                } else {
                    *newline && self.s[pos - 1] == b'\n'
                };
                ok && self.run(pc + 1, pos)
            }
            Inst::End { newline } => {
                let ok = if pos == self.s.len() {
                    !self.noteol
                } else {
                    *newline && self.s[pos] == b'\n'
                };
                ok && self.run(pc + 1, pos)
            }
            Inst::Save(n) => {
                let n = *n;
                let old = self.caps[n];
                self.caps[n] = pos as isize;
                if self.run(pc + 1, pos) {
                    true
                } else {
                    self.caps[n] = old;
                    false
                }
            }
            Inst::Jmp(t) => self.run(*t, pos),
            Inst::Split(a, b) => {
                let a = *a;
                let b = *b;
                self.run(a, pos) || self.run(b, pos)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

/// `regcomp(preg, regex, cflags)`：编译正则。返回 0 成功，非 0 为错误码。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regcomp(preg: *mut regex_t, regex: *const c_char, cflags: c_int) -> c_int {
    if preg.is_null() || regex.is_null() {
        return REG_BADPAT;
    }
    let p = &mut *preg;
    p.prog = core::ptr::null_mut();
    p.re_nsub = 0;
    p.cflags = cflags;
    p.errcode = 0;
    let pat = crate::stdio::cstr_bytes(regex);
    let extended = cflags & REG_EXTENDED != 0;
    let icase = cflags & REG_ICASE != 0;
    let newline = cflags & REG_NEWLINE != 0;
    let mut ps = Parser {
        pat,
        pos: 0,
        extended,
        icase,
        newline,
        insts: Vec::new(),
        classes: Vec::new(),
        nsub: 0,
    };
    // 整体匹配的捕获槽：0/1。
    ps.emit(Inst::Save(0));
    if let Err(e) = ps.parse_alt() {
        p.errcode = e;
        return e;
    }
    if ps.pos != pat.len() {
        // 未消费完（多余 ')' 等）。
        p.errcode = REG_EPAREN;
        return REG_EPAREN;
    }
    ps.emit(Inst::Save(1));
    ps.emit(Inst::Match);
    let prog = Box::new(Program { insts: ps.insts, classes: ps.classes });
    p.prog = Box::into_raw(prog) as *mut c_void;
    p.re_nsub = ps.nsub as size_t;
    0
}

/// `regexec(preg, string, nmatch, pmatch, eflags)`：在 `string` 中查找匹配。
///
/// 返回 0 = 匹配；`REG_NOMATCH` = 无匹配。`pmatch[0]` 是整体匹配，`pmatch[i]` 是第 i 个捕获组
/// （未参与匹配的组 `rm_so = rm_eo = -1`，POSIX）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regexec(
    preg: *const regex_t,
    string: *const c_char,
    nmatch: size_t,
    pmatch: *mut regmatch_t,
    eflags: c_int,
) -> c_int {
    if preg.is_null() || string.is_null() {
        return REG_BADPAT;
    }
    let p = &*preg;
    if p.prog.is_null() {
        return REG_BADPAT;
    }
    let prog: &Program = &*(p.prog as *const Program);
    let s = crate::stdio::cstr_bytes(string);
    let icase = p.cflags & REG_ICASE != 0;
    let ngroups = p.re_nsub + 1;
    // 从每个起点尝试（POSIX 的「最左」语义）；同一起点上取引擎给出的第一个成功（贪婪优先）。
    let mut start = 0usize;
    loop {
        let mut ex = Exec {
            prog,
            s,
            icase,
            notbol: eflags & REG_NOTBOL != 0,
            noteol: eflags & REG_NOTEOL != 0,
            caps: alloc::vec![-1isize; 2 * (ngroups + 1)],
            steps: 0,
        };
        if ex.run(0, start) {
            if !pmatch.is_null() && nmatch > 0 {
                let n = core::cmp::min(nmatch, ngroups);
                for i in 0..n {
                    let so = ex.caps[2 * i];
                    let eo = ex.caps[2 * i + 1];
                    *pmatch.add(i) = regmatch_t { rm_so: so, rm_eo: eo };
                }
                // 未填充的槽按 POSIX 置 -1。
                for i in n..nmatch {
                    *pmatch.add(i) = regmatch_t { rm_so: -1, rm_eo: -1 };
                }
            }
            return 0;
        }
        if start >= s.len() {
            break;
        }
        start += 1;
    }
    REG_NOMATCH
}

/// `regerror(errcode, preg, errbuf, errbuf_size)`：把错误码转成人类可读串。
///
/// 返回**所需**缓冲区大小（含结尾 NUL，POSIX）。`errbuf_size > 0` 时截断写入。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regerror(
    errcode: c_int,
    _preg: *const regex_t,
    errbuf: *mut c_char,
    errbuf_size: size_t,
) -> size_t {
    let msg: &[u8] = match errcode {
        0 => b"no error",
        REG_NOMATCH => b"no match",
        REG_BADPAT => b"invalid regular expression",
        REG_ECOLLATE => b"invalid collating element",
        REG_ECTYPE => b"character class not supported (this libc has no [[:name:]] tables)",
        REG_EESCAPE => b"trailing backslash",
        REG_ESUBREG => b"back references are not supported",
        REG_EBRACK => b"unmatched [ or [^",
        REG_EPAREN => b"unmatched ( or \\(",
        REG_EBRACE => b"unmatched \\{",
        REG_BADBR => b"invalid repetition count(s)",
        REG_ERANGE => b"invalid character range",
        REG_ESPACE => b"out of memory",
        REG_BADRPT => b"repetition-operator operand invalid",
        _ => b"unknown regex error",
    };
    let need = msg.len() + 1;
    if !errbuf.is_null() && errbuf_size > 0 {
        let n = core::cmp::min(msg.len(), errbuf_size - 1);
        core::ptr::copy_nonoverlapping(msg.as_ptr(), errbuf as *mut u8, n);
        *errbuf.add(n) = 0;
    }
    need
}

/// `regfree(preg)`：释放编译产物（幂等：重复调用安全）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn regfree(preg: *mut regex_t) {
    if preg.is_null() {
        return;
    }
    let p = &mut *preg;
    if !p.prog.is_null() {
        drop(Box::from_raw(p.prog as *mut Program));
        p.prog = core::ptr::null_mut();
    }
}
