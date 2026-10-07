//! 3P6-2 第二波（第四批）：C1 清单里「纯库代码 / 只差包装」的那一批。
//!
//! 纪律同前几批：每项先核实底层能力（不预猜），真实实现，诚实边界写在函数上。
//! 本批**不含**需要动 stdio 内部状态的两项（`tmpfile` 在 stdio.rs，`sigsetjmp`/`siglongjmp`
//! 在 setjmp.rs），也不含 `setvbuf`/`setbuf`——那两项经核实**无法忠实实现**，已从 C1
//! 撤回并登记为「不支持」（理由见 docs/TODO/libc-posix-surface.md）。

use crate::ctypes::{c_char, c_int, c_void, size_t};
use crate::errno::{set_errno, EINVAL};

// ===========================================================================
// memalign / valloc —— posix_memalign 的传统别名（POSIX 未收录，但现实代码大量使用）
// ===========================================================================

/// `memalign(alignment, size)`：分配 `alignment` 字节对齐的内存。
///
/// **底层能力已核实**：本 libc 的 `posix_memalign` 早已实现（libc/src/malloc.rs）。
/// POSIX 未收录 `memalign`（它是 BSD/glibc 扩展），故这里就是它的最薄别名——
/// 不另写一套对齐分配器（S15 单点）。
///
/// **诚实边界**：`alignment` 不是 2 的幂或不是 `sizeof(void*)` 的倍数时，
/// `posix_memalign` 会返回 `EINVAL`；此时本函数按传统语义返回 NULL 并置 errno
/// （glibc 的 memalign 同样返回 NULL）。
#[unsafe(no_mangle)]
pub extern "C" fn memalign(alignment: size_t, size: size_t) -> *mut c_void {
    let mut p: *mut u8 = core::ptr::null_mut();
    if crate::malloc::posix_memalign(&mut p, alignment, size) != 0 {
        return core::ptr::null_mut();
    }
    p as *mut c_void
}

/// `valloc(size)`：分配**页对齐**内存（传统 BSD 接口）。
///
/// 等价 `memalign(getpagesize(), size)`。`getpagesize()` 已在本 libc 实现（走内核真实页大小），
/// 故这里同样是最薄组合，不硬编码 4096。
#[unsafe(no_mangle)]
pub extern "C" fn valloc(size: size_t) -> *mut c_void {
    let ps = crate::unistd::getpagesize();
    if ps <= 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    memalign(ps as size_t, size)
}

// ===========================================================================
// fnmatch —— 文件名模式匹配（POSIX，纯库代码）
// ===========================================================================

/// `fnmatch` 标志（取值与 glibc/Linux 一致）。
pub const FNM_NOMATCH: c_int = 1;
pub const FNM_PATHNAME: c_int = 1 << 0;
pub const FNM_NOESCAPE: c_int = 1 << 1;
pub const FNM_PERIOD: c_int = 1 << 2;

/// 在 `s` 的 `[i..]` 上匹配 `p` 的 `[j..]`（递归回溯）。
///
/// 为什么用递归而不是手写状态机：模式语言的 `*`/`?`/`[...]` 组合只有回溯最直观，
/// 且本函数的模式长度有界（文件名模式，不是正则）。**代价如实声明**：模式里 `*` 很多时
/// 最坏是指数级——POSIX 未要求 fnmatch 有复杂度保证，glibc 同样用回溯。
fn fnm(p: &[u8], s: &[u8], flags: c_int) -> bool {
    let pathname = flags & FNM_PATHNAME != 0;
    let noescape = flags & FNM_NOESCAPE != 0;
    let period = flags & FNM_PERIOD != 0;
    fnm_at(p, s, flags, pathname, noescape, period, true)
}

#[allow(clippy::too_many_arguments)]
fn fnm_at(
    p: &[u8],
    s: &[u8],
    _flags: c_int,
    pathname: bool,
    noescape: bool,
    period: bool,
    mut at_start: bool,
) -> bool {
    let mut pi = 0usize;
    let mut si = 0usize;
    let mut star: Option<(usize, usize)> = None; // (pattern 位置 after '*', string 位置)
    while si < s.len() {
        let mut matched_len: Option<usize> = None;
        if pi < p.len() {
            match p[pi] {
                b'*' => {
                    // 折叠连续的 '*'。
                    let mut k = pi;
                    while k < p.len() && p[k] == b'*' {
                        k += 1;
                    }
                    // FNM_PERIOD：'*' 不匹配开头的 '.'（除非模式本身以 '.' 开头）。
                    if period && at_start && si == 0 && !s.is_empty() && s[0] == b'.' {
                        return false;
                    }
                    // FNM_PATHNAME：'*' 不跨 '/'。
                    star = Some((k, si));
                    pi = k;
                    if pi == p.len() {
                        // 尾随 '*'：PATHNAME 下不得跨 '/'。
                        return !pathname || !s[si..].contains(&b'/');
                    }
                    continue;
                }
                b'?' => {
                    if pathname && s[si] == b'/' {
                        return false;
                    }
                    matched_len = Some(1);
                }
                b'[' => {
                    if pathname && s[si] == b'/' {
                        return false;
                    }
                    if let Some((consumed, hit)) = match_bracket(&p[pi..], s[si], noescape) {
                        if hit {
                            matched_len = Some(consumed);
                        } else {
                            matched_len = None;
                        }
                    } else {
                        // '[' 未闭合：按字面 '[' 处理（glibc 同）。
                        if s[si] == b'[' {
                            matched_len = Some(1);
                        } else {
                            matched_len = None;
                        }
                    }
                }
                b'\\' if !noescape && pi + 1 < p.len() => {
                    if s[si] == p[pi + 1] {
                        matched_len = Some(2);
                    }
                }
                c => {
                    if s[si] == c {
                        matched_len = Some(1);
                    }
                }
            }
        }
        match matched_len {
            Some(n) => {
                pi += n;
                si += 1;
                at_start = false;
            }
            None => match star {
                Some((sp, ss)) => {
                    // 回溯：让 '*' 多吃一个字符（PATHNAME 下不跨 '/'）。
                    if pathname && s[ss] == b'/' {
                        return false;
                    }
                    star = Some((sp, ss + 1));
                    pi = sp;
                    si = ss + 1;
                    at_start = false;
                }
                None => return false,
            },
        }
    }
    // 串已耗尽：模式余下必须全是 '*'。
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// 匹配 `p[0] == '['` 开始的括号表达式。返回 `(消耗的模式字节数, 是否命中)`；
/// `None` = 表达式未闭合（调用方按字面 '[' 处理）。
fn match_bracket(p: &[u8], c: u8, noescape: bool) -> Option<(usize, bool)> {
    let mut i = 1usize;
    let mut negate = false;
    if i < p.len() && (p[i] == b'!' || p[i] == b'^') {
        negate = true;
        i += 1;
    }
    let mut hit = false;
    let mut first = true;
    loop {
        if i >= p.len() {
            return None; // 未闭合
        }
        let ch = p[i];
        if ch == b']' && !first {
            i += 1;
            break;
        }
        first = false;
        // 范围 a-z（转义后的 '-' 或末尾 '-' 不算范围）。
        let lo = if ch == b'\\' && !noescape && i + 1 < p.len() {
            i += 1;
            p[i]
        } else {
            ch
        };
        if i + 2 < p.len() && p[i + 1] == b'-' && p[i + 2] != b']' {
            let hi = if p[i + 2] == b'\\' && !noescape && i + 3 < p.len() {
                p[i + 3]
            } else {
                p[i + 2]
            };
            if lo <= c && c <= hi {
                hit = true;
            }
            i += if p[i + 2] == b'\\' && !noescape { 4 } else { 3 };
        } else {
            if lo == c {
                hit = true;
            }
            i += 1;
        }
    }
    Some((i, hit != negate))
}

/// `fnmatch(pattern, string, flags)`：shell 风格通配匹配。匹配返回 0，不匹配返回 `FNM_NOMATCH`。
///
/// 支持：`*`、`?`、`[...]`（含 `!`/`^` 取反与 `a-z` 范围）、`\\` 转义；
/// 标志：`FNM_PATHNAME`（`*`/`?`/`[]` 不跨 `/`）、`FNM_NOESCAPE`、`FNM_PERIOD`（开头 `.` 不被 `*` 匹配）。
///
/// **诚实边界**：`FNM_CASEFOLD`（GNU 扩展）**不支持**——传入未定义位**不静默忽略**，
/// 一律 `EINVAL` + 返回 `FNM_NOMATCH`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fnmatch(pattern: *const c_char, string: *const c_char, flags: c_int) -> c_int {
    if pattern.is_null() || string.is_null() {
        set_errno(EINVAL);
        return FNM_NOMATCH;
    }
    const KNOWN: c_int = FNM_PATHNAME | FNM_NOESCAPE | FNM_PERIOD;
    if flags & !KNOWN != 0 {
        set_errno(EINVAL);
        return FNM_NOMATCH;
    }
    let p = crate::stdio::cstr_bytes(pattern);
    let s = crate::stdio::cstr_bytes(string);
    if fnm(p, s, flags) { 0 } else { FNM_NOMATCH }
}

// ===========================================================================
// getopt / getopt_long —— 命令行解析（POSIX + GNU 扩展，纯库代码）
// ===========================================================================

/// `optarg`：当前选项的参数（`optstring` 里该选项后带 `:` 时非空）。
#[unsafe(no_mangle)]
pub static mut optarg: *mut c_char = core::ptr::null_mut();
/// `optind`：下一个待处理参数的索引（POSIX 要求初值 1）。
#[unsafe(no_mangle)]
pub static mut optind: c_int = 1;
/// `opterr`：非 0 时 getopt 自行打印错误信息。
#[unsafe(no_mangle)]
pub static mut opterr: c_int = 1;
/// `optopt`：出错的那个选项字符。
#[unsafe(no_mangle)]
pub static mut optopt: c_int = 0;

static mut OPT_POS: usize = 1; // 当前参数内的扫描位置（1 = 跳过 argv[i][0] 的 '-'）
/// 上一次扫描时的 `optind`。用于检测「调用方换了 argv 数组」并重置扫描位置。
static mut OPT_LAST_IND: c_int = 0;

/// `struct option`（getopt_long 的长选项描述，C 布局）。
#[repr(C)]
pub struct option {
    pub name: *const c_char,
    pub has_arg: c_int,
    pub flag: *mut c_int,
    pub val: c_int,
}

// 名字刻意保持 C 头文件里的小写拼写（`getopt.h` 的 POSIX/GNU 约定），故显式关掉命名风格检查。
#[allow(non_upper_case_globals)]
pub const no_argument: c_int = 0;
#[allow(non_upper_case_globals)]
pub const required_argument: c_int = 1;
#[allow(non_upper_case_globals)]
pub const optional_argument: c_int = 2;

/// 在 `optstring` 里查字符 `c`，返回 `(是否已知, 参数类型)`。
fn opt_lookup(optstring: &[u8], c: u8) -> (bool, c_int) {
    let mut i = 0usize;
    while i < optstring.len() {
        if optstring[i] == c {
            let has = if i + 1 < optstring.len() && optstring[i + 1] == b':' {
                if i + 2 < optstring.len() && optstring[i + 2] == b':' { optional_argument } else { required_argument }
            } else {
                no_argument
            };
            return (true, has);
        }
        i += 1;
    }
    (false, no_argument)
}

/// 取下一个短选项（供 getopt 与 getopt_long 共用，S15 单点）。
///
/// 返回 `(选项字符, 参数, 参数是否来自本 argv 项) `；`None` = 选项结束。
unsafe fn short_next(
    argc: c_int,
    argv: *const *mut c_char,
    optstring: &[u8],
) -> Option<(c_int, *mut c_char, bool)> {
    loop {
        if optind as usize >= argc as usize {
            return None;
        }
        // **每次 `optind` 变化都必须重置扫描位置**：`optind` 是调用方可写的全局，
        // 连续对**不同** argv 数组调用 getopt（自检与库代码常见）时，若沿用上一次的
        // OPT_POS 就会从参数中间开始扫——那是只在第二次调用才显形的缺陷。
        if OPT_LAST_IND != optind {
            OPT_POS = 1;
            OPT_LAST_IND = optind;
        }
        let cur = *argv.add(optind as usize);
        if cur.is_null() {
            return None;
        }
        let item = crate::stdio::cstr_bytes(cur);
        if OPT_POS == 0 {
            OPT_POS = 1;
        }
        // 非选项（不以 '-' 开头，或就是 "-"）→ 停止（经典非置换语义）。
        if item.len() < 2 || item[0] != b'-' {
            return None;
        }
        if item.len() == 2 && item[1] == b'-' {
            // "--" 终止选项解析。
            optind += 1;
            OPT_POS = 1;
            return None;
        }
        if OPT_POS >= item.len() {
            optind += 1;
            OPT_POS = 1;
            continue;
        }
        let c = item[OPT_POS];
        let (known, has_arg) = opt_lookup(optstring, c);
        if !known {
            optopt = c as c_int;
            OPT_POS += 1;
            if OPT_POS >= item.len() {
                optind += 1;
                OPT_POS = 1;
            }
            return Some((b'?' as c_int, core::ptr::null_mut(), false));
        }
        if has_arg == no_argument {
            OPT_POS += 1;
            if OPT_POS >= item.len() {
                optind += 1;
                OPT_POS = 1;
            }
            return Some((c as c_int, core::ptr::null_mut(), false));
        }
        // 需要参数：先看同一 argv 项内剩余字符，否则取下一项。
        if OPT_POS + 1 < item.len() {
            let arg = cur.add(OPT_POS + 1);
            optind += 1;
            OPT_POS = 1;
            return Some((c as c_int, arg, true));
        }
        if optind as usize + 1 >= argc as usize {
            // 缺参数。
            optopt = c as c_int;
            optind += 1;
            OPT_POS = 1;
            if has_arg == required_argument {
                return Some((if optstring.first() == Some(&b':') { b':' as c_int } else { b'?' as c_int }, core::ptr::null_mut(), false));
            }
            return Some((c as c_int, core::ptr::null_mut(), false));
        }
        let arg = *argv.add(optind as usize + 1);
        optind += 2;
        OPT_POS = 1;
        return Some((c as c_int, arg, true));
    }
}

/// `getopt(argc, argv, optstring)`：解析短选项。
///
/// 语义按 POSIX：`optstring` 中 `:` 表示需要参数、`::` 表示可选参数（GNU 扩展）；
/// 未知选项返回 `'?'` 并置 `optopt`；缺参数返回 `':'`（当 `optstring` 以 `:` 开头）否则 `'?'`。
///
/// **诚实边界（S09）**：**不做参数置换**（GNU 的「把非选项挪到末尾」）——遇到第一个
/// 非选项即停止。这是 POSIX 允许的经典行为，但 `prog -a file -b` 里的 `-b` 不会被解析；
/// 需要置换的调用方请显式排序或改用 `getopt_long`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getopt(argc: c_int, argv: *const *mut c_char, optstring: *const c_char) -> c_int {
    if argv.is_null() || optstring.is_null() {
        return -1;
    }
    let os = crate::stdio::cstr_bytes(optstring);
    match short_next(argc, argv, os) {
        Some((c, arg, has)) => {
            optarg = arg;
            let _ = has;
            c
        }
        None => -1,
    }
}

/// `getopt_long(argc, argv, optstring, longopts, longindex)`：长选项 + 短选项。
///
/// `--name`、`--name=value`、`--name value` 三种形态都支持；唯一前缀缩写**支持**
/// （与 glibc 同：前缀不唯一返回 `'?'`）。`longindex` 非空时回写命中的长选项下标。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getopt_long(
    argc: c_int,
    argv: *const *mut c_char,
    optstring: *const c_char,
    longopts: *const option,
    longindex: *mut c_int,
) -> c_int {
    if argv.is_null() || optstring.is_null() {
        return -1;
    }
    let os = crate::stdio::cstr_bytes(optstring);
    loop {
        if optind as usize >= argc as usize {
            return -1;
        }
        let cur = *argv.add(optind as usize);
        if cur.is_null() {
            return -1;
        }
        let item = crate::stdio::cstr_bytes(cur);
        if item.len() < 2 || item[0] != b'-' || item[1] != b'-' {
            // 不是长选项：交给短选项路径（含非选项终止）。
            if OPT_POS == 0 {
                OPT_POS = 1;
            }
            let is_short = item.len() >= 2 && item[0] == b'-' && item[1] != b'-';
            if !is_short {
                return -1;
            }
            return match short_next(argc, argv, os) {
                Some((c, arg, _)) => {
                    optarg = arg;
                    c
                }
                None => -1,
            };
        }
        if item.len() == 2 {
            // "--" 终止。
            optind += 1;
            return -1;
        }
        // 切出名字与内联值。
        let body = &item[2..];
        let (name, inline_val): (&[u8], Option<&[u8]>) = match body.iter().position(|&b| b == b'=') {
            Some(eq) => (&body[..eq], Some(&body[eq + 1..])),
            None => (body, None),
        };
        // 在前缀匹配里找唯一命中。
        let mut found: Option<usize> = None;
        let mut ambiguous = false;
        let mut exact = false;
        let mut i = 0usize;
        if !longopts.is_null() {
            loop {
                let opt = &*longopts.add(i);
                if opt.name.is_null() {
                    break;
                }
                let on = crate::stdio::cstr_bytes(opt.name);
                if on == name {
                    found = Some(i);
                    exact = true;
                    break;
                }
                if on.len() > name.len() && &on[..name.len()] == name {
                    if found.is_some() {
                        ambiguous = true;
                    } else {
                        found = Some(i);
                    }
                }
                i += 1;
            }
        }
        if found.is_none() || ambiguous {
            optind += 1;
            optopt = 0;
            if opterr != 0 {
                let _ = libsys::write(2, b"unrecognized option: --");
                let _ = libsys::write(2, name);
                let _ = libsys::write(2, b"\n");
            }
            return b'?' as c_int;
        }
        let idx = found.unwrap();
        if !longindex.is_null() {
            *longindex = idx as c_int;
        }
        let opt = &*longopts.add(idx);
        let _ = exact;
        let arg: *mut c_char = match opt.has_arg {
            no_argument => {
                if inline_val.is_some() {
                    optind += 1;
                    if opterr != 0 {
                        let _ = libsys::write(2, b"option does not take an argument: --");
                        let _ = libsys::write(2, name);
                        let _ = libsys::write(2, b"\n");
                    }
                    return b'?' as c_int;
                }
                optind += 1;
                core::ptr::null_mut()
            }
            optional_argument => {
                optind += 1;
                match inline_val {
                    Some(_) => cur.add(2 + name.len() + 1),
                    None => core::ptr::null_mut(),
                }
            }
            _ => match inline_val {
                Some(_) => {
                    optind += 1;
                    cur.add(2 + name.len() + 1)
                }
                None => {
                    if optind as usize + 1 >= argc as usize {
                        optind += 1;
                        if opterr != 0 {
                            let _ = libsys::write(2, b"option requires an argument: --");
                            let _ = libsys::write(2, name);
                            let _ = libsys::write(2, b"\n");
                        }
                        return b'?' as c_int;
                    }
                    let a = *argv.add(optind as usize + 1);
                    optind += 2;
                    a
                }
            },
        };
        optarg = arg;
        if !opt.flag.is_null() {
            *opt.flag = opt.val;
            return 0;
        }
        return opt.val;
    }
}

// ===========================================================================
// strptime —— 时间串解析（POSIX，纯库代码）
// ===========================================================================

const MONTHS: [&[u8]; 12] = [
    b"January", b"February", b"March", b"April", b"May", b"June",
    b"July", b"August", b"September", b"October", b"November", b"December",
];
const WDAYS: [&[u8]; 7] = [
    b"Sunday", b"Monday", b"Tuesday", b"Wednesday", b"Thursday", b"Friday", b"Saturday",
];

fn parse_uint(s: &[u8], i: &mut usize, max_digits: usize) -> Option<i64> {
    let start = *i;
    let mut v: i64 = 0;
    let mut n = 0usize;
    while *i < s.len() && n < max_digits && s[*i].is_ascii_digit() {
        v = v * 10 + (s[*i] - b'0') as i64;
        *i += 1;
        n += 1;
    }
    if *i == start { None } else { Some(v) }
}

/// 跳过输入里的空白（POSIX：格式串里的空白匹配任意量空白，含零）。
fn skip_ws(s: &[u8], i: &mut usize) {
    while *i < s.len() && (s[*i] == b' ' || s[*i] == b'\t' || s[*i] == b'\n' || s[*i] == b'\r') {
        *i += 1;
    }
}

/// 大小写不敏感前缀匹配（返回消耗的字节数）。
fn ci_prefix(s: &[u8], i: usize, word: &[u8], full_only: bool) -> Option<usize> {
    let n = word.len();
    if i + n > s.len() {
        // 允许缩写：尝试更短的前缀（至少 3 字符，与 glibc 的 %b 缩写一致）。
        if full_only {
            return None;
        }
        let mut k = s.len() - i;
        while k >= 3 {
            if s[i..i + k].eq_ignore_ascii_case(&word[..k]) {
                return Some(k);
            }
            k -= 1;
        }
        return None;
    }
    if s[i..i + n].eq_ignore_ascii_case(word) {
        return Some(n);
    }
    if !full_only {
        let mut k = n - 1;
        while k >= 3 {
            if s[i..i + k].eq_ignore_ascii_case(&word[..k]) {
                return Some(k);
            }
            k -= 1;
        }
    }
    None
}

/// `strptime(buf, format, tm)`：按 `format` 解析 `buf` 填入 `tm`。
///
/// 支持：`%Y %y %m %d %e %H %I %M %S %p %b %B %h %a %A %j %n %t %%` 与空白；
/// `%Y` 写 `tm_year = 年 - 1900`（POSIX），`%y` 按 69/70 分界映射 19xx/20xx。
///
/// **诚实边界（S09）**：以下说明符**未实现**，遇到时**原样当字面量匹配**并如实返回失败
/// （绝不静默跳过——静默跳过会让调用方以为解析成功）：
/// `%c %x %X %U %W %V %G %g %s %z %Z %F %T %D %R %C %k %l`。
/// 返回值：成功返回 `buf` 中**第一个未消耗字符**的指针；失败返回 NULL（**不置 errno**，POSIX 未要求）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strptime(buf: *const c_char, format: *const c_char, tm: *mut crate::time::Tm) -> *mut c_char {
    if buf.is_null() || format.is_null() || tm.is_null() {
        return core::ptr::null_mut();
    }
    let s = crate::stdio::cstr_bytes(buf);
    let f = crate::stdio::cstr_bytes(format);
    let t = &mut *tm;
    // POSIX：未指定的字段**保持不变**（调用方先填默认值），故这里不预置任何字段。
    let mut si = 0usize;
    let mut fi = 0usize;
    let mut pm: Option<bool> = None; // Some(true)=PM
    let mut hour12: Option<i64> = None;
    while fi < f.len() {
        let c = f[fi];
        if c == b'%' {
            fi += 1;
            if fi >= f.len() {
                return core::ptr::null_mut();
            }
            let spec = f[fi];
            fi += 1;
            match spec {
                b'Y' => match parse_uint(s, &mut si, 4) {
                    Some(v) => t.tm_year = (v - 1900) as i32,
                    None => return core::ptr::null_mut(),
                },
                b'y' => match parse_uint(s, &mut si, 2) {
                    Some(v) => t.tm_year = (if v < 69 { v + 100 } else { v }) as i32,
                    None => return core::ptr::null_mut(),
                },
                b'm' => match parse_uint(s, &mut si, 2) {
                    Some(v) if (1..=12).contains(&v) => t.tm_mon = (v - 1) as i32,
                    _ => return core::ptr::null_mut(),
                },
                b'd' | b'e' => {
                    skip_ws(s, &mut si);
                    match parse_uint(s, &mut si, 2) {
                        Some(v) if (1..=31).contains(&v) => t.tm_mday = v as i32,
                        _ => return core::ptr::null_mut(),
                    }
                }
                b'H' => match parse_uint(s, &mut si, 2) {
                    Some(v) if v <= 23 => t.tm_hour = v as i32,
                    _ => return core::ptr::null_mut(),
                },
                b'I' => match parse_uint(s, &mut si, 2) {
                    Some(v) if (1..=12).contains(&v) => hour12 = Some(v),
                    _ => return core::ptr::null_mut(),
                },
                b'M' => match parse_uint(s, &mut si, 2) {
                    Some(v) if v <= 59 => t.tm_min = v as i32,
                    _ => return core::ptr::null_mut(),
                },
                b'S' => match parse_uint(s, &mut si, 2) {
                    Some(v) if v <= 60 => t.tm_sec = v as i32,
                    _ => return core::ptr::null_mut(),
                },
                b'p' | b'P' => {
                    if ci_prefix(s, si, b"AM", true).is_some() {
                        pm = Some(false);
                        si += 2;
                    } else if ci_prefix(s, si, b"PM", true).is_some() {
                        pm = Some(true);
                        si += 2;
                    } else {
                        return core::ptr::null_mut();
                    }
                }
                b'b' | b'h' => {
                    let mut hit = false;
                    for (m, name) in MONTHS.iter().enumerate() {
                        if let Some(n) = ci_prefix(s, si, *name, false) {
                            t.tm_mon = m as i32;
                            si += n;
                            hit = true;
                            break;
                        }
                    }
                    if !hit {
                        return core::ptr::null_mut();
                    }
                }
                b'B' => {
                    let mut hit = false;
                    for (m, name) in MONTHS.iter().enumerate() {
                        if ci_prefix(s, si, *name, true).is_some() {
                            t.tm_mon = m as i32;
                            si += name.len();
                            hit = true;
                            break;
                        }
                    }
                    if !hit {
                        return core::ptr::null_mut();
                    }
                }
                b'a' | b'A' => {
                    let mut hit = false;
                    for name in WDAYS.iter() {
                        if let Some(n) = ci_prefix(s, si, *name, false) {
                            si += n;
                            hit = true;
                            break;
                        }
                    }
                    if !hit {
                        return core::ptr::null_mut();
                    }
                }
                b'j' => match parse_uint(s, &mut si, 3) {
                    Some(v) if (1..=366).contains(&v) => t.tm_yday = (v - 1) as i32,
                    _ => return core::ptr::null_mut(),
                },
                b'n' | b't' => skip_ws(s, &mut si),
                b'%' => {
                    if si >= s.len() || s[si] != b'%' {
                        return core::ptr::null_mut();
                    }
                    si += 1;
                }
                _ => return core::ptr::null_mut(),
            }
            continue;
        }
        if c.is_ascii_whitespace() {
            // 格式串里的空白匹配任意量空白（含零）。
            while fi < f.len() && f[fi].is_ascii_whitespace() {
                fi += 1;
            }
            skip_ws(s, &mut si);
            continue;
        }
        if si >= s.len() || s[si] != c {
            return core::ptr::null_mut();
        }
        si += 1;
        fi += 1;
    }
    // %I + %p 合成 24 小时制（POSIX：两者需一起用才确定小时）。
    if let Some(h) = hour12 {
        let h24 = match pm {
            Some(true) => if h == 12 { 12 } else { h + 12 },
            Some(false) => if h == 12 { 0 } else { h },
            None => h, // 无 %p：按 12 小时制的字面值（POSIX 未规定，如实取原值）
        };
        t.tm_hour = h24 as i32;
    }
    buf.add(si) as *mut c_char
}
