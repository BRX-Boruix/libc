//! 3P6-2 第二波（第五批）：C2 清单（需先核实底层能力）**核实后确实可实现**的项。
//!
//! 核实结论与逐项理由见 `docs/TODO/libc-posix-surface.md`：C2 的 18 项里，
//! **8 项可实现**（本文件 + `dirent.rs` 的 `seekdir`/`telldir`/`scandir`/`alphasort`），
//! **10 项经核实不支持**并移入 B 类（`ioctl`/`fsync`/`fchmod`/`fchown`/`ttyname`/`alarm`/
//! `sigpending`/`sigsuspend`/`sigaltstack`/`confstr`/`nice`）。
//!
//! 纪律同前几批：每项先核实数据源/底层能力（不预猜），真实实现，诚实边界写在函数上。

use alloc::vec::Vec;

use crate::ctypes::{c_char, c_int, size_t};
use crate::errno::{set_errno, EINVAL};

// ===========================================================================
// uname / gethostname —— 系统标识（真值来自内核 INFO_VERSION）
// ===========================================================================

/// `struct utsname`（POSIX；字段顺序与 `libc/include/sys/utsname.h` 一致，各 65 字节）。
#[repr(C)]
pub struct utsname {
    pub sysname: [c_char; 65],
    pub nodename: [c_char; 65],
    pub release: [c_char; 65],
    pub version: [c_char; 65],
    pub machine: [c_char; 65],
}

fn set_field(dst: &mut [c_char; 65], s: &[u8]) {
    let n = s.len().min(64);
    for i in 0..n {
        dst[i] = s[i] as c_char;
    }
    dst[n] = 0;
}

/// 把非负整数写成十进制（返回写入长度）。用于拼 `M.m.p` 版本串。
fn dec_into(buf: &mut [u8], mut v: u64) -> usize {
    if v == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0usize;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

/// `uname(buf)`：系统标识（POSIX）。成功返回 0，失败 -1 置 errno。
///
/// **每个字段的真值来源（S09：逐字段说明，不编造）**：
///
/// | 字段 | 值 | 来源 |
/// | --- | --- | --- |
/// | `sysname` | `Boruix` | 系统名（事实陈述） |
/// | `nodename` | **空串** | 本系统**没有主机名概念**——内核与 libsys 都没有该数据源（已核实）。**不编 `localhost`** |
/// | `release` | `0.1.0` | 内核 `INFO_VERSION`（`libsys::info`，值 `0x000100`） |
/// | `version` | 同 `release` | 本系统**没有独立的构建标识串**（`INFO_*` 只有上面那个版本号），如实复用 |
/// | `machine` | `x86_64` | 目标架构是**编译期事实**（本 libc 只构建 x86_64） |
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uname(buf: *mut utsname) -> c_int {
    if buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let u = unsafe { &mut *buf };
    set_field(&mut u.sysname, b"Boruix");
    set_field(&mut u.nodename, b"");
    let v = libsys::info(libsys::nr::INFO_VERSION).unwrap_or(0);
    let mut rb = [0u8; 24];
    let mut n = dec_into(&mut rb, (v >> 16) & 0xff);
    rb[n] = b'.';
    n += 1;
    n += dec_into(&mut rb[n..], (v >> 8) & 0xff);
    rb[n] = b'.';
    n += 1;
    n += dec_into(&mut rb[n..], v & 0xff);
    set_field(&mut u.release, &rb[..n]);
    set_field(&mut u.version, &rb[..n]);
    set_field(&mut u.machine, b"x86_64");
    0
}

/// `gethostname(name, len)`：取主机名（POSIX）。成功返回 0，失败 -1 置 errno。
///
/// **诚实边界**：本系统**没有主机名**（同 `uname` 的 `nodename`，已核实无数据源）。
/// POSIX 只要求「写入一个以 NUL 结尾的字符串」，故这里**如实写空串**——
/// 编一个 `localhost` 会让调用方以为系统配了主机名。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gethostname(name: *mut c_char, len: size_t) -> c_int {
    if name.is_null() || len == 0 {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { *name = 0 };
    0
}

/// `getlogin()`：返回登录名（POSIX）。取不到返回 NULL（POSIX 允许）。
///
/// 实现：查环境变量 `LOGNAME`，其次 `USER`（POSIX 认可的通行做法）。
///
/// **诚实边界**：本系统**内核/libsys 没有「登录会话名」查询**（已核实：只有 `identity_query`
/// 的 uid/gid/caps）。环境里也没有时**如实返回 NULL**，绝不编一个名字。
/// 返回指向**静态缓冲**的指针（后续调用会覆盖，POSIX 允许）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getlogin() -> *mut c_char {
    static mut BUF: [c_char; 64] = [0; 64];
    for key in [b"LOGNAME\0".as_ptr(), b"USER\0".as_ptr()] {
        let v = unsafe { crate::stdlib::getenv(key as *const c_char) };
        if v.is_null() || unsafe { *v } == 0 {
            continue;
        }
        let b = unsafe { crate::stdio::cstr_bytes(v) };
        let n = b.len().min(63);
        let buf = unsafe { &mut *core::ptr::addr_of_mut!(BUF) };
        for i in 0..n {
            buf[i] = b[i] as c_char;
        }
        buf[n] = 0;
        return buf.as_mut_ptr();
    }
    core::ptr::null_mut()
}

// ===========================================================================
// wordexp / wordfree —— shell 风格的词展开（**无命令替换**）
// ===========================================================================

pub const WRDE_APPEND: c_int = 1;
pub const WRDE_DOOFFS: c_int = 2;
pub const WRDE_NOCMD: c_int = 4;
pub const WRDE_REUSE: c_int = 8;
pub const WRDE_SHOWERR: c_int = 16;
pub const WRDE_UNDEF: c_int = 32;

pub const WRDE_BADCHAR: c_int = 1;
pub const WRDE_BADVAL: c_int = 2;
pub const WRDE_CMDSUB: c_int = 3;
pub const WRDE_NOSPACE: c_int = 4;
pub const WRDE_SYNTAX: c_int = 6;

/// `wordexp_t`（POSIX；字段与 `libc/include/wordexp.h` 一致）。
#[repr(C)]
pub struct wordexp_t {
    pub we_wordc: size_t,
    pub we_wordv: *mut *mut c_char,
    pub we_offs: size_t,
}

/// 展开 `$NAME` / `${NAME}`；返回新的扫描位置。
unsafe fn we_expand_var(words: &[u8], i: usize, cur: &mut Vec<u8>, flags: c_int) -> Result<usize, c_int> {
    let mut j = i + 1;
    if j >= words.len() {
        return Err(WRDE_SYNTAX);
    }
    // `$(` 是命令替换：本系统**没有 shell 可执行它**，按 POSIX 用 WRDE_CMDSUB 如实报告
    // （这正是该错误码的用途），而不是静默展开成空。
    if words[j] == b'(' {
        return Err(WRDE_CMDSUB);
    }
    let braced = words[j] == b'{';
    if braced {
        j += 1;
    }
    let start = j;
    while j < words.len() {
        let c = words[j];
        let ok = if braced {
            c != b'}'
        } else {
            c == b'_' || c.is_ascii_alphanumeric()
        };
        if !ok {
            break;
        }
        j += 1;
    }
    if start == j {
        return Err(WRDE_SYNTAX);
    }
    let name = &words[start..j];
    if braced {
        if j >= words.len() || words[j] != b'}' {
            return Err(WRDE_SYNTAX);
        }
        j += 1;
    }
    let mut nb: Vec<u8> = Vec::with_capacity(name.len() + 1);
    nb.extend_from_slice(name);
    nb.push(0);
    let v = unsafe { crate::stdlib::getenv(nb.as_ptr() as *const c_char) };
    if v.is_null() {
        // WRDE_UNDEF：未定义变量视为错误；否则按 POSIX 展开为空串。
        if flags & WRDE_UNDEF != 0 {
            return Err(WRDE_BADVAL);
        }
    } else {
        cur.extend_from_slice(unsafe { crate::stdio::cstr_bytes(v) });
    }
    Ok(j)
}

/// 把 `words` 切成词并做变量展开 / 去引号。返回 `Err(错误码)` 表示展开失败。
unsafe fn we_expand(words: &[u8], flags: c_int) -> Result<Vec<Vec<u8>>, c_int> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut has_cur = false;
    let mut i = 0usize;
    while i < words.len() {
        let c = words[i];
        match c {
            b' ' | b'\t' | b'\n' => {
                if has_cur {
                    out.push(core::mem::take(&mut cur));
                    has_cur = false;
                }
                i += 1;
            }
            b'\'' => {
                has_cur = true;
                i += 1;
                let start = i;
                while i < words.len() && words[i] != b'\'' {
                    i += 1;
                }
                if i >= words.len() {
                    return Err(WRDE_SYNTAX);
                }
                cur.extend_from_slice(&words[start..i]);
                i += 1;
            }
            b'"' => {
                has_cur = true;
                i += 1;
                while i < words.len() && words[i] != b'"' {
                    if words[i] == b'\\' && i + 1 < words.len() {
                        let n = words[i + 1];
                        // 双引号内只有 $ ` " \ 与换行受反斜杠转义（POSIX）。
                        if n == b'$' || n == b'`' || n == b'"' || n == b'\\' || n == b'\n' {
                            cur.push(n);
                            i += 2;
                            continue;
                        }
                    }
                    if words[i] == b'$' {
                        i = unsafe { we_expand_var(words, i, &mut cur, flags) }?;
                        continue;
                    }
                    if words[i] == b'`' {
                        return Err(WRDE_CMDSUB);
                    }
                    cur.push(words[i]);
                    i += 1;
                }
                if i >= words.len() {
                    return Err(WRDE_SYNTAX);
                }
                i += 1;
            }
            b'\\' => {
                has_cur = true;
                if i + 1 >= words.len() {
                    return Err(WRDE_SYNTAX);
                }
                cur.push(words[i + 1]);
                i += 2;
            }
            b'$' => {
                has_cur = true;
                i = unsafe { we_expand_var(words, i, &mut cur, flags) }?;
            }
            b'`' => return Err(WRDE_CMDSUB),
            // 未加引号的 shell 元字符：本系统**没有 shell** 可执行它们 ⇒ 按 POSIX 用
            // WRDE_BADCHAR 如实报告（**不静默当字面量**——那会让调用方以为管道被处理了）。
            b'|' | b'&' | b';' | b'<' | b'>' | b'(' | b')' | b'{' | b'}' => {
                return Err(WRDE_BADCHAR)
            }
            _ => {
                has_cur = true;
                cur.push(c);
                i += 1;
            }
        }
    }
    if has_cur {
        out.push(cur);
    }
    Ok(out)
}

/// `wordexp(words, p, flags)`：shell 风格的词展开（POSIX）。
///
/// 支持：空白分词、单/双引号、反斜杠转义、`$VAR` / `${VAR}`、`~`（见下）、
/// 以及**路径名展开**（含 `*?[` 的词走本 libc 的 `glob`；无匹配时按 POSIX 原样保留该词）。
///
/// **诚实边界（S09，逐条）**：
///  - **命令替换**（`$(...)`、反引号）**不支持** ⇒ 如实返回 `WRDE_CMDSUB`（POSIX 为此定义了该错误码），
///    绝不静默展开成空串。
///  - **未加引号的 shell 元字符**（`| & ; < > ( ) { }`）⇒ `WRDE_BADCHAR`（本系统没有 shell）。
///  - **`~` 不做家目录展开**：本系统没有 `/users/<name>` → 家目录的映射约定（用户表在
///    `/system/info/users`，但是只读 JSON 视图，且 `HOME` 由 login 注入）。`~` 按字面保留，
///    **不假装展开**。
///  - 不支持 `WRDE_SHOWERR`（不打印到 stderr——本函数不写任何输出）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wordexp(words: *const c_char, p: *mut wordexp_t, flags: c_int) -> c_int {
    if words.is_null() || p.is_null() {
        return WRDE_NOSPACE;
    }
    let w = unsafe { &mut *p };
    if flags & WRDE_REUSE != 0 {
        unsafe { wordfree(p) };
    }
    let src = unsafe { crate::stdio::cstr_bytes(words) };
    let toks = match unsafe { we_expand(src, flags) } {
        Ok(t) => t,
        Err(e) => return e,
    };
    let mut final_words: Vec<Vec<u8>> = Vec::new();
    for t in toks.iter() {
        let magic = t.iter().any(|&c| c == b'*' || c == b'?' || c == b'[');
        if !magic {
            final_words.push(t.clone());
            continue;
        }
        let mut pat: Vec<u8> = t.clone();
        pat.push(0);
        let mut g = crate::glob::glob_t {
            gl_pathc: 0,
            gl_pathv: core::ptr::null_mut(),
            gl_offs: 0,
        };
        let r = unsafe { crate::glob::glob(pat.as_ptr() as *const c_char, 0, None, &mut g) };
        if r == 0 && g.gl_pathc > 0 {
            for k in 0..g.gl_pathc {
                let s = unsafe { *g.gl_pathv.add(k) };
                if !s.is_null() {
                    final_words.push(unsafe { crate::stdio::cstr_bytes(s) }.to_vec());
                }
            }
            unsafe { crate::glob::globfree(&mut g) };
        } else {
            // POSIX：无匹配时该词**原样保留**（不做删除）。
            final_words.push(t.clone());
        }
    }
    let offs = if flags & WRDE_DOOFFS != 0 { w.we_offs } else { 0 };
    let mut base: Vec<Vec<u8>> = Vec::new();
    if flags & WRDE_APPEND != 0 && !w.we_wordv.is_null() {
        for k in 0..w.we_wordc {
            let s = unsafe { *w.we_wordv.add(offs + k) };
            if !s.is_null() {
                base.push(unsafe { crate::stdio::cstr_bytes(s) }.to_vec());
            }
        }
        crate::malloc::free(w.we_wordv as *mut u8);
    }
    base.extend(final_words.into_iter());
    let total = offs + base.len();
    let arr = crate::malloc::malloc((total + 1) * core::mem::size_of::<*mut c_char>()) as *mut *mut c_char;
    if arr.is_null() {
        return WRDE_NOSPACE;
    }
    for k in 0..offs {
        unsafe { *arr.add(k) = core::ptr::null_mut() };
    }
    for (k, s) in base.iter().enumerate() {
        let m = crate::malloc::malloc(s.len() + 1) as *mut c_char;
        if m.is_null() {
            for j in 0..k {
                crate::malloc::free(unsafe { *arr.add(offs + j) } as *mut u8);
            }
            crate::malloc::free(arr as *mut u8);
            return WRDE_NOSPACE;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), m as *mut u8, s.len());
            *m.add(s.len()) = 0;
            *arr.add(offs + k) = m;
        }
    }
    unsafe { *arr.add(total) = core::ptr::null_mut() };
    w.we_wordv = arr;
    w.we_wordc = base.len();
    if flags & WRDE_DOOFFS == 0 {
        w.we_offs = 0;
    }
    0
}

/// `wordfree(p)`：释放 `wordexp` 分配的字符串与数组（幂等）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wordfree(p: *mut wordexp_t) {
    if p.is_null() || unsafe { (*p).we_wordv.is_null() } {
        return;
    }
    let w = unsafe { &mut *p };
    let offs = w.we_offs;
    for k in 0..w.we_wordc {
        let s = unsafe { *w.we_wordv.add(offs + k) };
        if !s.is_null() {
            crate::malloc::free(s as *mut u8);
        }
    }
    crate::malloc::free(w.we_wordv as *mut u8);
    w.we_wordv = core::ptr::null_mut();
    w.we_wordc = 0;
}

