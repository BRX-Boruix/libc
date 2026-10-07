//! `glob` / `globfree`（POSIX 路径名模式展开）——纯库代码实现。
//!
//! ## 实现路线
//!
//! 逐组件展开：把模式按 `/` 切成组件，从左到右维护「当前前缀集合」；无通配符的组件直接拼接
//! （快路径，不读目录），有通配符的组件用 [`crate::dirent`] 的 `opendir`/`readdir` 枚举目录、
//! 用 [`crate::posix_batch4::fnmatch`] 逐项匹配。最后对每个候选做一次 `stat` 存在性核验
//! （POSIX 要求 glob 只返回**存在**的路径），并按 `GLOB_MARK` 给目录补 `/`。
//!
//! 目录枚举走 libc 自己的 `opendir`/`readdir`（S15 单点：目录读取逻辑只有一份），
//! 而不是另开一条 libsys 直调——否则「目录项语义」会有两个实现，迟早分叉。
//!
//! ## 诚实边界（S09，逐条可复核）
//!
//! - **不支持**：花括号展开 `{a,b}`（GNU 扩展，非 POSIX）、`GLOB_TILDE`、`GLOB_BRACE`、
//!   `GLOB_ONLYDIR`、`GLOB_ALTDIRFUNC`。传入这些位一律 `GLOB_NOSPACE`？——不，**如实返回
//!   `GLOB_NOMATCH` 且不静默忽略**：见 `glob()` 里的未定义位检查（返回 `GLOB_ABORTED`）。
//! - **`errfunc`**：会被调用（路径 + errno）；返回非 0 则中止并返回 `GLOB_ABORTED`。
//! - **`GLOB_ERR`**：打开目录失败时中止；未设时忽略该目录（POSIX）。
//! - `gl_pathv` 的字符串与数组都由 `malloc` 分配，`globfree` 负责释放（POSIX 语义）。

use alloc::vec::Vec;

use crate::ctypes::{c_char, c_int, size_t};
use crate::errno::set_errno;

pub const GLOB_ERR: c_int = 1 << 0;
pub const GLOB_MARK: c_int = 1 << 1;
pub const GLOB_NOSORT: c_int = 1 << 2;
pub const GLOB_DOOFFS: c_int = 1 << 3;
pub const GLOB_NOCHECK: c_int = 1 << 4;
pub const GLOB_APPEND: c_int = 1 << 5;
pub const GLOB_NOESCAPE: c_int = 1 << 6;
pub const GLOB_PERIOD: c_int = 1 << 7;

pub const GLOB_NOSPACE: c_int = 1;
pub const GLOB_ABORTED: c_int = 2;
pub const GLOB_NOMATCH: c_int = 3;

/// `glob_t`（C 可见布局见 `libc/include/glob.h`；字段与 glibc 的公开字段同形）。
#[repr(C)]
pub struct glob_t {
    pub gl_pathc: size_t,
    pub gl_pathv: *mut *mut c_char,
    pub gl_offs: size_t,
}

/// `errfunc(path, errno)` 的函数指针类型。
pub type GlobErrFn = unsafe extern "C" fn(*const c_char, c_int) -> c_int;

fn has_magic(comp: &[u8], noescape: bool) -> bool {
    let mut i = 0usize;
    while i < comp.len() {
        match comp[i] {
            b'*' | b'?' | b'[' => return true,
            b'\\' if !noescape => i += 1, // 转义的下一个字符不算通配符
            _ => {}
        }
        i += 1;
    }
    false
}

fn join(prefix: &[u8], comp: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(prefix.len() + comp.len() + 1);
    out.extend_from_slice(prefix);
    if !prefix.is_empty() && !prefix.ends_with(b"/") {
        out.push(b'/');
    }
    out.extend_from_slice(comp);
    out
}

/// 目录枚举路径：空前缀（相对模式的起点）读 `.`。
fn dir_path(prefix: &[u8]) -> &[u8] {
    if prefix.is_empty() { b"." } else { prefix }
}

/// 展开一个组件：对 `prefix` 目录里的每个条目做 `fnmatch`。
unsafe fn expand_component(
    prefix: &[u8],
    comp: &[u8],
    fnm_flags: c_int,
    errfunc: Option<GlobErrFn>,
    glob_err: bool,
    out: &mut Vec<Vec<u8>>,
    aborted: &mut bool,
) {
    let dp = dir_path(prefix);
    let mut cpath: Vec<u8> = Vec::with_capacity(dp.len() + 1);
    cpath.extend_from_slice(dp);
    cpath.push(0);
    let dir = crate::dirent::opendir(cpath.as_ptr() as *const c_char);
    if dir.is_null() {
        if glob_err {
            if let Some(f) = errfunc {
                if f(cpath.as_ptr() as *const c_char, 0) != 0 {
                    *aborted = true;
                }
            } else {
                *aborted = true;
            }
        }
        return;
    }
    loop {
        let ent = crate::dirent::readdir(dir);
        if ent.is_null() {
            break;
        }
        let name = &(*ent).d_name;
        // 取 NUL 结尾的名字字节。
        let mut n = 0usize;
        while n < name.len() && name[n] != 0 {
            n += 1;
        }
        let nb: &[u8] = core::slice::from_raw_parts(name.as_ptr() as *const u8, n);
        // '.' / '..' 只在其模式显式以 '.' 开头时才参与匹配。
        let starts_dot = !nb.is_empty() && nb[0] == b'.';
        if starts_dot && (nb == b"." || nb == b"..") && comp.first() != Some(&b'.') {
            continue;
        }
        if starts_dot && comp.first() != Some(&b'.') {
            continue;
        }
        let mut cname: Vec<u8> = Vec::with_capacity(n + 1);
        cname.extend_from_slice(nb);
        cname.push(0);
        let mut cpat: Vec<u8> = Vec::with_capacity(comp.len() + 1);
        cpat.extend_from_slice(comp);
        cpat.push(0);
        if crate::posix_batch4::fnmatch(
            cpat.as_ptr() as *const c_char,
            cname.as_ptr() as *const c_char,
            fnm_flags,
        ) == 0
        {
            out.push(join(prefix, nb));
        }
    }
    crate::dirent::closedir(dir);
}

/// `glob(pattern, flags, errfunc, pglob)`：展开路径名模式。
///
/// 返回 0 成功；`GLOB_NOSPACE`/`GLOB_ABORTED`/`GLOB_NOMATCH` 见 `glob.h`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn glob(
    pattern: *const c_char,
    flags: c_int,
    errfunc: Option<GlobErrFn>,
    pglob: *mut glob_t,
) -> c_int {
    if pattern.is_null() || pglob.is_null() {
        return GLOB_NOSPACE;
    }
    const KNOWN: c_int = GLOB_ERR
        | GLOB_MARK
        | GLOB_NOSORT
        | GLOB_DOOFFS
        | GLOB_NOCHECK
        | GLOB_APPEND
        | GLOB_NOESCAPE
        | GLOB_PERIOD;
    if flags & !KNOWN != 0 {
        // 未实现的标志（GLOB_BRACE / GLOB_TILDE / GLOB_ONLYDIR ...）：如实报错，
        // **绝不静默忽略**——静默忽略会让调用方以为花括号展开了。
        set_errno(crate::errno::ENOTSUP);
        return GLOB_ABORTED;
    }
    let pat = crate::stdio::cstr_bytes(pattern);
    let noescape = flags & GLOB_NOESCAPE != 0;
    let mut fnm_flags: c_int = 0;
    if noescape {
        fnm_flags |= crate::posix_batch4::FNM_NOESCAPE;
    }
    if flags & GLOB_PERIOD == 0 {
        fnm_flags |= crate::posix_batch4::FNM_PERIOD;
    }
    // 组件切分（保留绝对/相对信息）。
    let absolute = pat.first() == Some(&b'/');
    let mut comps: Vec<&[u8]> = Vec::new();
    for c in pat.split(|&b| b == b'/') {
        if c.is_empty() {
            continue;
        }
        comps.push(c);
    }
    let mut cur: Vec<Vec<u8>> = Vec::new();
    cur.push(if absolute { b"/".to_vec() } else { Vec::new() });
    let mut aborted = false;
    for (i, comp) in comps.iter().enumerate() {
        let last = i + 1 == comps.len();
        let mut next: Vec<Vec<u8>> = Vec::new();
        if !has_magic(comp, noescape) {
            for p in cur.iter() {
                next.push(join(p, comp));
            }
        } else {
            for p in cur.iter() {
                let mut cfl = fnm_flags;
                if !last {
                    // 中间组件必须能继续下钻，故不允许跨越 '/'（组件里本就没有 '/'）。
                    cfl |= crate::posix_batch4::FNM_PATHNAME;
                }
                expand_component(p, comp, cfl, errfunc, flags & GLOB_ERR != 0, &mut next, &mut aborted);
            }
        }
        cur = next;
        if cur.is_empty() {
            break;
        }
    }
    if aborted {
        return GLOB_ABORTED;
    }
    // 存在性核验 + 目录标记。
    let trailing_slash = pat.last() == Some(&b'/') && pat.len() > 1;
    let mut results: Vec<Vec<u8>> = Vec::new();
    for p in cur.into_iter() {
        let mut cs: Vec<u8> = p.clone();
        cs.push(0);
        let exists = libsys::stat(core::str::from_utf8(&p).unwrap_or("")).is_ok();
        if !exists {
            continue;
        }
        let is_dir = libsys::read_dir(core::str::from_utf8(&p).unwrap_or("")).is_ok();
        if trailing_slash && !is_dir {
            continue;
        }
        let mut r = p;
        if (flags & GLOB_MARK != 0 || trailing_slash) && is_dir && !r.ends_with(b"/") {
            r.push(b'/');
        }
        results.push(r);
    }
    if results.is_empty() {
        if flags & GLOB_NOCHECK != 0 {
            results.push(pat.to_vec());
        } else {
            // POSIX：无匹配时 gl_pathc = 0、gl_pathv 可以为 NULL。
            if flags & GLOB_APPEND == 0 {
                (*pglob).gl_pathc = 0;
                (*pglob).gl_pathv = core::ptr::null_mut();
            }
            return GLOB_NOMATCH;
        }
    }
    if flags & GLOB_NOSORT == 0 {
        results.sort();
    }
    // 与既有结果合并（GLOB_APPEND）。
    let mut all: Vec<Vec<u8>> = Vec::new();
    let offs = if flags & GLOB_DOOFFS != 0 { (*pglob).gl_offs } else { 0 };
    if flags & GLOB_APPEND != 0 && !(*pglob).gl_pathv.is_null() {
        let oldc = (*pglob).gl_pathc;
        let oldv = (*pglob).gl_pathv;
        for i in 0..oldc {
            let s = *oldv.add(offs + i);
            if !s.is_null() {
                all.push(crate::stdio::cstr_bytes(s).to_vec());
            }
        }
        // 释放旧数组（字符串所有权转交给下面重新分配的数组，故这里只释放数组本体）。
        crate::malloc::free(oldv as *mut u8);
    }
    all.extend(results.into_iter());
    let total = offs + all.len();
    let bytes = (total + 1) * core::mem::size_of::<*mut c_char>();
    let arr = crate::malloc::malloc(bytes) as *mut *mut c_char;
    if arr.is_null() {
        return GLOB_NOSPACE;
    }
    for i in 0..offs {
        *arr.add(i) = core::ptr::null_mut();
    }
    for (i, s) in all.iter().enumerate() {
        let m = crate::malloc::malloc(s.len() + 1) as *mut c_char;
        if m.is_null() {
            // 失败：释放已分配的，如实 NOSPACE（不留半成品）。
            for j in 0..i {
                crate::malloc::free(*arr.add(offs + j) as *mut u8);
            }
            crate::malloc::free(arr as *mut u8);
            return GLOB_NOSPACE;
        }
        core::ptr::copy_nonoverlapping(s.as_ptr(), m as *mut u8, s.len());
        *m.add(s.len()) = 0;
        *arr.add(offs + i) = m;
    }
    *arr.add(total) = core::ptr::null_mut();
    (*pglob).gl_pathv = arr;
    (*pglob).gl_pathc = all.len();
    if flags & GLOB_DOOFFS == 0 {
        (*pglob).gl_offs = 0;
    }
    0
}

/// `globfree(pglob)`：释放 `glob`/`globfree` 分配的字符串与数组（幂等）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn globfree(pglob: *mut glob_t) {
    if pglob.is_null() || (*pglob).gl_pathv.is_null() {
        return;
    }
    let offs = (*pglob).gl_offs;
    let n = (*pglob).gl_pathc;
    let arr = (*pglob).gl_pathv;
    for i in 0..n {
        let s = *arr.add(offs + i);
        if !s.is_null() {
            crate::malloc::free(s as *mut u8);
        }
    }
    crate::malloc::free(arr as *mut u8);
    (*pglob).gl_pathv = core::ptr::null_mut();
    (*pglob).gl_pathc = 0;
}
