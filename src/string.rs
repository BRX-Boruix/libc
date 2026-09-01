//! 内存与字符串函数（C ABI）。
//!
//! 全部为纯逻辑、可移植（S01/S04），不依赖任何系统调用。这些函数在 host
//! 上亦可编译运行（用于 unit test，S23）。边界语义（S19）：
//! - \`memcpy\`/\`memset\` 以字节处理，长度 size_t 无符号，无溢出路径；
//! - \`strlen\` 依赖 NUL 终止，调用方须保证终止符存在（C 契约）；
//! - 所有指针必须有效，本层不校验（C 契约，与 libc 一致）。

use crate::ctypes::{size_t, c_char, c_int};

/// \`memcpy(dst, src, n)\`：复制 n 字节。src/dst 不得重叠（重叠用 memmove）。
///
/// 注意：\`memcpy\`/\`memmove\`/\`memset\`/\`memcmp\` 的 C ABI 符号由 libsys（builtins）
/// 提供，本 libc 不重复导出（S09，避免与编译内建符号冲突）。此处为纯 Rust 别名
/// 供 Rust 侧调用。
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, n: size_t) -> *mut u8 {
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, n);
    }
    dst
}

/// \`memmove(dst, src, n)\`：复制 n 字节，允许 src/dst 重叠。
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: size_t) -> *mut u8 {
    unsafe {
        core::ptr::copy(src, dst, n);
    }
    dst
}

/// \`memset(s, c, n)\`：把 s 前 n 字节设为 c（低 8 位）。
pub unsafe extern "C" fn memset(s: *mut u8, c: c_int, n: size_t) -> *mut u8 {
    unsafe {
        core::ptr::write_bytes(s, c as u8, n);
    }
    s
}

/// \`memcmp(a, b, n)\`：比较前 n 字节。返回 <0/0/>0（按无符号字节序）。
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: size_t) -> c_int {
    unsafe {
        for i in 0..n {
            let x = *a.add(i);
            let y = *b.add(i);
            if x != y {
                return (x as c_int) - (y as c_int);
            }
        }
    }
    0
}

/// \`memchr(s, c, n)\`：在前 n 字节中查找 c，返回首次出现位置或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memchr(s: *const u8, c: c_int, n: size_t) -> *mut u8 {
    let target = c as u8;
    unsafe {
        for i in 0..n {
            if *s.add(i) == target {
                return s.add(i) as *mut u8;
            }
        }
    }
    core::ptr::null_mut()
}

/// \`strlen(s)\`：返回字符串长度（不含 NUL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(s: *const c_char) -> size_t {
    unsafe {
        let mut p = s;
        while *p != 0 {
            p = p.add(1);
        }
        p as usize - s as usize
    }
}

/// \`strnlen(s, max)\`：返回长度，最多 \`max\`（遇到 NUL 停止）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strnlen(s: *const c_char, max: size_t) -> size_t {
    unsafe {
        let mut n = 0;
        while n < max && *s.add(n) != 0 {
            n += 1;
        }
        n
    }
}

/// \`strcmp(a, b)\`：按无符号字符序比较，返回 <0/0/>0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    unsafe {
        let mut i = 0;
        loop {
            let x = *a.add(i) as u8;
            let y = *b.add(i) as u8;
            if x != y {
                return (x as c_int) - (y as c_int);
            }
            if x == 0 {
                return 0;
            }
            i += 1;
        }
    }
}

/// \`strncmp(a, b, n)\`：比较至多 n 字节。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(a: *const c_char, b: *const c_char, n: size_t) -> c_int {
    unsafe {
        for i in 0..n {
            let x = *a.add(i) as u8;
            let y = *b.add(i) as u8;
            if x != y {
                return (x as c_int) - (y as c_int);
            }
            if x == 0 {
                return 0;
            }
        }
    }
    0
}

/// \`strcpy(dst, src)\`：复制字符串（含 NUL）。dst 须足够大。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    unsafe {
        let mut i = 0;
        loop {
            let c = *src.add(i);
            *dst.add(i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
    }
    dst
}

/// \`strncpy(dst, src, n)\`：复制至多 n 字节。
///
/// 语义（C11 7.24.2.4）：若 src 长度 < n，剩余字节补 NUL；若 src 长度 >= n，
/// 复制 n 字节且**不**写终止符（调用方须自行保证）。返回 dst。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy(dst: *mut c_char, src: *const c_char, n: size_t) -> *mut c_char {
    unsafe {
        let mut i = 0;
        // 复制 src 中的字符，直到 NUL 或 n 耗尽。
        while i < n {
            let c = *src.add(i);
            if c == 0 {
                break;
            }
            *dst.add(i) = c;
            i += 1;
        }
        // 若 src 提前 NUL，补 NUL 填充剩余。
        while i < n {
            *dst.add(i) = 0;
            i += 1;
        }
    }
    dst
}


/// \`strcat(dst, src)\`：把 src 追加到 dst 末尾（含 NUL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    unsafe {
        let dlen = strlen(dst);
        let d = dst.add(dlen) as *mut u8;
        let s = src as *const u8;
        let mut i = 0;
        loop {
            let c = *s.add(i);
            *d.add(i) = c;
            if c == 0 {
                break;
            }
            i += 1;
        }
    }
    dst
}

/// \`strchr(s, c)\`：查找字符 c（含 NUL 处），返回指针或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(s: *const c_char, c: c_int) -> *mut c_char {
    let target = c as c_char;
    unsafe {
        let mut i = 0;
        loop {
            let ch = *s.add(i);
            if ch == target {
                return s.add(i) as *mut c_char;
            }
            if ch == 0 {
                return core::ptr::null_mut();
            }
            i += 1;
        }
    }
}

/// \`strrchr(s, c)\`：查找字符 c 最后一次出现，返回指针或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    let target = c as c_char;
    unsafe {
        let len = strlen(s);
        let mut i = len;
        loop {
            if *s.add(i) == target {
                return s.add(i) as *mut c_char;
            }
            if i == 0 {
                return core::ptr::null_mut();
            }
            i -= 1;
        }
    }
}

/// \`strstr(haystack, needle)\`：在 haystack 中查找子串 needle，返回位置或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    unsafe {
        let nlen = strlen(needle);
        if nlen == 0 {
            return haystack as *mut c_char;
        }
        let hlen = strlen(haystack);
        if nlen > hlen {
            return core::ptr::null_mut();
        }
        let mut i = 0;
        while i + nlen <= hlen {
            if strncmp(haystack.add(i), needle, nlen) == 0 {
                return haystack.add(i) as *mut c_char;
            }
            i += 1;
        }
        core::ptr::null_mut()
    }
}

/// \`strdup(s)\`：复制字符串到新分配内存。失败返回 NULL 置 ENOMEM。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strdup(s: *const c_char) -> *mut c_char {
    unsafe {
        let len = strlen(s);
        let p = crate::malloc::malloc(len + 1) as *mut c_char;
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(s, p, len + 1);
        }
        p
    }
}
/// \`strspn(s, accept)\`：返回 s 开头连续由 accept 中字符组成的长度。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strspn(s: *const c_char, accept: *const c_char) -> size_t {
    unsafe {
        if s.is_null() || accept.is_null() {
            return 0;
        }
        let mut n = 0usize;
        loop {
            let c = *s.add(n);
            if c == 0 {
                break;
            }
            // 是否在 accept 中。
            let mut j = 0usize;
            let mut found = false;
            loop {
                let a = *accept.add(j);
                if a == 0 {
                    break;
                }
                if a == c {
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                break;
            }
            n += 1;
        }
        n
    }
}

/// \`strcspn(s, reject)\`：返回 s 开头连续**不在** reject 中的字符数。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcspn(s: *const c_char, reject: *const c_char) -> size_t {
    unsafe {
        if s.is_null() || reject.is_null() {
            return 0;
        }
        let mut n = 0usize;
        loop {
            let c = *s.add(n);
            if c == 0 {
                break;
            }
            // 是否在 reject 中。
            let mut j = 0usize;
            let mut found = false;
            loop {
                let r = *reject.add(j);
                if r == 0 {
                    break;
                }
                if r == c {
                    found = true;
                    break;
                }
                j += 1;
            }
            if found {
                break;
            }
            n += 1;
        }
        n
    }
}

/// \`strpbrk(s, accept)\`：返回 s 中第一个出现在 accept 中的字符指针，无则 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strpbrk(s: *const c_char, accept: *const c_char) -> *mut c_char {
    unsafe {
        if s.is_null() || accept.is_null() {
            return core::ptr::null_mut();
        }
        let mut i = 0usize;
        loop {
            let c = *s.add(i);
            if c == 0 {
                return core::ptr::null_mut();
            }
            let mut j = 0usize;
            loop {
                let a = *accept.add(j);
                if a == 0 {
                    break;
                }
                if a == c {
                    return (s as *mut c_char).add(i);
                }
                j += 1;
            }
            i += 1;
        }
    }
}

/// \`strncat(dst, src, n)\`：把 src 至多 n 个字符追加到 dst 末尾并 NUL 终止。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncat(dst: *mut c_char, src: *const c_char, n: size_t) -> *mut c_char {
    unsafe {
        if dst.is_null() || src.is_null() {
            return dst;
        }
        // 找 dst 末尾。
        let mut dlen = 0usize;
        while *dst.add(dlen) != 0 {
            dlen += 1;
        }
        let mut k = 0usize;
        while k < n && *src.add(k) != 0 {
            *dst.add(dlen + k) = *src.add(k);
            k += 1;
        }
        *dst.add(dlen + k) = 0;
        dst
    }
}

/// \`strtok_r(s, delim, saveptr)\`：按分隔符切分字符串（线程安全版）。
///
/// 调用方提供 \`saveptr\`（指向保存切分位置的指针），故无内部静态状态，可并发
/// 用于多个字符串/线程。首次调用传 s，后续传 NULL 继续。\`saveptr\` 非空必须有效。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok_r(s: *mut c_char, delim: *const c_char, saveptr: *mut *mut c_char) -> *mut c_char {
    unsafe {
        if saveptr.is_null() {
            return core::ptr::null_mut();
        }
        let cur = if s.is_null() {
            *saveptr
        } else {
            s
        };
        if cur.is_null() {
            return core::ptr::null_mut();
        }
        // 跳过前导分隔符。
        let mut start = cur;
        loop {
            let c = *start;
            if c == 0 {
                *saveptr = core::ptr::null_mut();
                return core::ptr::null_mut();
            }
            let mut is_delim = false;
            let mut j = 0usize;
            loop {
                let d = *delim.add(j);
                if d == 0 {
                    break;
                }
                if d == c {
                    is_delim = true;
                    break;
                }
                j += 1;
            }
            if !is_delim {
                break;
            }
            start = start.add(1);
        }
        // 找到 token 末尾。
        let mut end = start;
        loop {
            let c = *end;
            if c == 0 {
                *saveptr = core::ptr::null_mut();
                return start;
            }
            let mut is_delim = false;
            let mut j = 0usize;
            loop {
                let d = *delim.add(j);
                if d == 0 {
                    break;
                }
                if d == c {
                    is_delim = true;
                    break;
                }
                j += 1;
            }
            if is_delim {
                break;
            }
            end = end.add(1);
        }
        *end = 0; // 终止 token。
        *saveptr = end.add(1);
        start
    }
}

/// \`strtok(s, delim)\`：按分隔符切分字符串（非线程安全，基于全局静态）。
///
/// **线程限制（S19 如实）**：用全局静态保存指针，仅适合单线程用户态；
/// 多线程/并发请用 \`strtok_r\`（线程安全，显式 saveptr）。首次调用传 s，
/// 后续传 NULL 继续。
static STRTOK_SAVE: core::sync::atomic::AtomicPtr<c_char> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtok(s: *mut c_char, delim: *const c_char) -> *mut c_char {
    unsafe {
        // 用 saveptr 指向全局静态的地址作为 strtok_r 的 saveptr。
        let mut tmp = STRTOK_SAVE.load(core::sync::atomic::Ordering::SeqCst);
        let r = strtok_r(s, delim, &mut tmp);
        STRTOK_SAVE.store(tmp, core::sync::atomic::Ordering::SeqCst);
        r
    }
}
