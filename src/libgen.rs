//! `<libgen.h>`：路径分解（POSIX）。
//!
//! **来路（3P6-2 第二波「整项缺失」类）**：由 `libc/tools/audit_posix_surface.py` 的反向对账
//! 列出。这两项是纯字符串处理，无系统调用依赖。
//!
//! ## 语义（POSIX.1-2008）与两处诚实说明
//!
//! - `basename(path)`：返回最后一个路径分量；`"/"` → `"/"`，空串/全斜杠以外的空结果 → `"."`。
//!   **本实现不改写调用方的字符串**（POSIX 允许改写，glibc 会改写；不改写更安全——传字符串
//!   字面量不会崩），返回的是**指向调用方字符串内部**的指针（POSIX 允许）。
//! - `dirname(path)`：返回去掉最后分量的目录部分；无斜杠 → `"."`。
//!   目录部分**不是**输入的后缀，故必须写入静态缓冲（POSIX 明确允许返回静态存储）。
//!   **诚实边界（S09）**：静态缓冲 ⇒ **不可重入/非线程安全**，且下次调用会覆盖——与 POSIX
//!   对 dirname 的允许一致，但调用方必须知道。

use crate::ctypes::c_char;

/// 静态缓冲：dirname 的返回值所在（POSIX 允许）。
static mut DIRNAME_BUF: [c_char; 4096] = [0; 4096];

fn bytes_len(p: *const c_char) -> usize {
    let mut n = 0usize;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// basename(path)：最后一个路径分量（见模块说明）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn basename(path: *mut c_char) -> *mut c_char {
    unsafe {
        if path.is_null() {
            return b".\0".as_ptr() as *mut c_char;
        }
        let n = bytes_len(path);
        // 去掉尾部斜杠。
        let mut end = n;
        while end > 0 && *path.add(end - 1) == b'/' as c_char {
            end -= 1;
        }
        if end == 0 {
            // 全斜杠（含空串）：POSIX 对空串给 "."，对全斜杠给 "/"。
            return if n == 0 {
                b".\0".as_ptr() as *mut c_char
            } else {
                b"/\0".as_ptr() as *mut c_char
            };
        }
        // 找最后一个分量起点。
        let mut start = end;
        while start > 0 && *path.add(start - 1) != b'/' as c_char {
            start -= 1;
        }
        if end == n {
            // 无尾部斜杠：结果**就是**输入的后缀，直接返回指向输入内部的指针（不改写、不复制）。
            path.add(start)
        } else {
            // 有尾部斜杠：结果**不是**输入的后缀（那个斜杠不是 NUL），必须复制到静态缓冲。
            //
            // **为什么不像 glibc 那样改写输入**：POSIX 允许改写（故传字符串字面量本就是 UB），
            // 但"不改写"对调用方更安全、也更符合本实现写在头文件里的承诺。代价是该情形下
            // 返回值位于静态缓冲 ⇒ **不可重入**（已写进头文件边界）。
            static mut BASENAME_BUF: [c_char; 4096] = [0; 4096];
            let buf = core::ptr::addr_of_mut!(BASENAME_BUF) as *mut c_char;
            let len = (end - start).min(4095);
            core::ptr::copy_nonoverlapping(path.add(start), buf, len);
            *buf.add(len) = 0;
            buf
        }
    }
}

/// dirname(path)：去掉最后一个分量后的目录部分（见模块说明）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dirname(path: *mut c_char) -> *mut c_char {
    unsafe {
        let buf = core::ptr::addr_of_mut!(DIRNAME_BUF) as *mut c_char;
        if path.is_null() {
            core::ptr::copy_nonoverlapping(b".\0".as_ptr() as *const c_char, buf, 2);
            return buf;
        }
        let n = bytes_len(path);
        let mut end = n;
        while end > 0 && *path.add(end - 1) == b'/' as c_char {
            end -= 1;
        }
        if end == 0 {
            // 无有效分量：空串/全斜杠 -> "." 或 "/"。
            let s: &[u8] = if n == 0 { b".\0" } else { b"/\0" };
            core::ptr::copy_nonoverlapping(s.as_ptr() as *const c_char, buf, s.len());
            return buf;
        }
        let mut i = end;
        while i > 0 && *path.add(i - 1) != b'/' as c_char {
            i -= 1;
        }
        if i == 0 {
            core::ptr::copy_nonoverlapping(b".\0".as_ptr() as *const c_char, buf, 2);
            return buf;
        }
        // 去掉目录部分尾部斜杠（但保留唯一的根斜杠）。
        let mut d = i;
        while d > 1 && *path.add(d - 1) == b'/' as c_char {
            d -= 1;
        }
        let len = d.min(4095);
        core::ptr::copy_nonoverlapping(path, buf, len);
        *buf.add(len) = 0;
        buf
    }
}
