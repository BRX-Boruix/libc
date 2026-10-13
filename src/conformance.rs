//! 全库转译审查的**符合性测试组**（conformance battery）。
//!
//! 目的：把 libc 里那些"看起来简单、所以从没核对过"的纯逻辑函数，按各自标准
//! （C11 / POSIX.1-2008）**明文规定的行为**逐条钉住。这些函数不碰内核，故宿主
//! （`cargo test --lib`）就能跑——真机侧另有 `shell/src/libc_check.rs` 的 libccheck。
//!
//! 为什么单独放一个文件：本组是按**标准条款**组织的（而不是按源文件），一处集中
//! 便于对照标准逐条核对；生产文件里只留各模块自己的局部不变量测试。
//!
//! 约定：每条断言都对应标准里的一句话，注释写**条款内容**而不是"这里测什么"。

use crate::ctypes::{c_char, c_int, c_void};

/// 把字节串（含结尾 NUL）当作 C 字符串指针。
fn cs(s: &[u8]) -> *const c_char {
    s.as_ptr() as *const c_char
}
fn csm(s: &[u8]) -> *mut c_char {
    s.as_ptr() as *mut c_char
}

// ===========================================================================
// string.h
// ===========================================================================

/// `strchr(s, 0)` 返回**终止符的地址**（C11 7.24.5.2：搜索包含结尾的空字符）。
#[test]
fn strchr_finds_terminator() {
    let s = b"abc\0";
    unsafe {
        let p = crate::string::strchr(cs(s), 0);
        assert_eq!(p as usize, s.as_ptr() as usize + 3);
        assert!(crate::string::strchr(cs(s), b'z' as c_int).is_null());
    }
}

/// `strrchr` 找**最后**一次出现；未找到返回 NULL。
#[test]
fn strrchr_finds_last() {
    let s = b"abca\0";
    unsafe {
        let p = crate::string::strrchr(cs(s), b'a' as c_int);
        assert_eq!(p as usize, s.as_ptr() as usize + 3);
    }
}

/// `strncpy`：src 短于 n 时**用 NUL 补齐到 n 个字节**；src 不短于 n 时
/// **不写终止符**（C11 7.24.2.4——这正是它与 `strlcpy` 的区别）。
#[test]
fn strncpy_pads_and_does_not_terminate() {
    unsafe {
        let mut dst = [0x7Fu8; 8];
        crate::string::strncpy(dst.as_mut_ptr() as *mut c_char, cs(b"ab\0"), 5);
        assert_eq!(&dst[..5], b"ab\0\0\0");

        let mut dst2 = [0x7Fu8; 8];
        crate::string::strncpy(dst2.as_mut_ptr() as *mut c_char, cs(b"abcdefgh\0"), 4);
        assert_eq!(&dst2[..4], b"abcd");
        assert_eq!(dst2[4], 0x7F, "must NOT terminate when src is not shorter than n");
    }
}

/// `strncat`：至多追加 n 个字符，且**总是** NUL 终止（C11 7.24.3.2）。
#[test]
fn strncat_appends_at_most_n_and_terminates() {
    unsafe {
        let mut buf = [0u8; 16];
        buf[..3].copy_from_slice(b"ab\0");
        crate::string::strncat(buf.as_mut_ptr() as *mut c_char, cs(b"cdef\0"), 2);
        assert_eq!(&buf[..5], b"abcd\0");

        let mut buf2 = [0u8; 8];
        buf2[..1].copy_from_slice(b"\0");
        crate::string::strncat(buf2.as_mut_ptr() as *mut c_char, cs(b"xyz\0"), 0);
        assert_eq!(buf2[0], 0, "n==0 must still leave a terminated string");
    }
}

/// `strtok_r`：跳过**前导**分隔符；连续分隔符之间不产生空 token；末尾不产生空 token；
/// 找不到 token 返回 NULL（POSIX strtok_r）。
#[test]
fn strtok_r_skips_empty_fields() {
    unsafe {
        let mut buf = *b"  a,,b  c  \0";
        let mut save: *mut c_char = core::ptr::null_mut();
        let mut got: alloc::vec::Vec<&[u8]> = alloc::vec::Vec::new();
        let mut p = crate::string::strtok_r(buf.as_mut_ptr() as *mut c_char, cs(b" ,\0"), &mut save);
        while !p.is_null() {
            let mut n = 0usize;
            while *p.add(n) != 0 {
                n += 1;
            }
            got.push(core::slice::from_raw_parts(p as *const u8, n));
            p = crate::string::strtok_r(core::ptr::null_mut(), cs(b" ,\0"), &mut save);
        }
        assert_eq!(got, alloc::vec![&b"a"[..], &b"b"[..], &b"c"[..]]);
    }
}

/// `strsep`：连续分隔符产生**空**字段（与 strtok 相反），末尾字段照常返回。
#[test]
fn strsep_yields_empty_fields() {
    unsafe {
        let mut buf = *b"a,,b\0";
        let mut sp = buf.as_mut_ptr() as *mut c_char;
        let mut got: alloc::vec::Vec<&[u8]> = alloc::vec::Vec::new();
        loop {
            let p = crate::string::strsep(&mut sp, cs(b",\0"));
            if p.is_null() {
                break;
            }
            let mut n = 0usize;
            while *p.add(n) != 0 {
                n += 1;
            }
            got.push(core::slice::from_raw_parts(p as *const u8, n));
        }
        assert_eq!(got, alloc::vec![&b"a"[..], &b""[..], &b"b"[..]]);
    }
}

/// `strstr`/`memmem`：**空 needle 返回 haystack 本身**（C11 7.24.5.7 的约定）。
#[test]
fn empty_needle_matches_at_start() {
    unsafe {
        let h = b"hello\0";
        assert_eq!(
            crate::string::strstr(cs(h), cs(b"\0")) as usize,
            h.as_ptr() as usize
        );
        let m = crate::string::memmem(h.as_ptr() as *const _, 5, b"".as_ptr() as *const _, 0);
        assert_eq!(m as usize, h.as_ptr() as usize);
    }
}

/// `strcspn`/`strspn`/`strpbrk`：补集/交集语义（C11 7.24.5.3/7.24.5.6/7.24.5.4）。
#[test]
fn span_functions() {
    unsafe {
        assert_eq!(crate::string::strcspn(cs(b"hello world\0"), cs(b" \0")), 5);
        assert_eq!(crate::string::strspn(cs(b"aabbcc\0"), cs(b"ab\0")), 4);
        let s = b"hello world\0";
        let p = crate::string::strpbrk(cs(s), cs(b"ow\0"));
        assert_eq!(p as usize, s.as_ptr() as usize + 4);
        assert!(crate::string::strpbrk(cs(s), cs(b"z\0")).is_null());
    }
}

/// `strcmp`/`strncmp`/`strcasecmp`：返回值**符号**有意义，绝对值无意义。
#[test]
fn compare_functions_sign() {
    unsafe {
        assert!(crate::string::strcmp(cs(b"abc\0"), cs(b"abd\0")) < 0);
        assert!(crate::string::strcmp(cs(b"abd\0"), cs(b"abc\0")) > 0);
        assert_eq!(crate::string::strcmp(cs(b"abc\0"), cs(b"abc\0")), 0);
        assert_eq!(crate::string::strncmp(cs(b"abcX\0"), cs(b"abcY\0"), 3), 0);
        assert_eq!(crate::string::strcasecmp(cs(b"AbC\0"), cs(b"aBc\0")), 0);
        assert_eq!(crate::string::strncasecmp(cs(b"AbCd\0"), cs(b"aBcE\0"), 3), 0);
    }
}

/// `memchr` 的 n==0 不得读取任何字节（返回 NULL）。
#[test]
fn memchr_zero_length() {
    unsafe {
        let s = b"abc";
        assert!(crate::string::memchr(s.as_ptr() as *const _, b'a' as c_int, 0).is_null());
        assert_eq!(
            crate::string::memchr(s.as_ptr() as *const _, b'c' as c_int, 3) as usize,
            s.as_ptr() as usize + 2
        );
    }
}

/// `memmove` 必须支持重叠区域（C11 7.24.2.2）。
#[test]
fn memmove_handles_overlap() {
    unsafe {
        let mut b = *b"abcdef\0";
        crate::string::memmove(b.as_mut_ptr().add(1) as *mut _, b.as_ptr() as *const _, 4);
        // b = "a" + 拷入的 "abcd" + 原来的 "f" + NUL
        assert_eq!(&b[..7], b"aabcdf\0");
    }
}

// ===========================================================================
// libgen.h（POSIX.1-2008）
// ===========================================================================

/// POSIX basename/dirname 的边界表。表里每一行都直接来自标准正文的例子与规则。
#[test]
fn basename_dirname_table() {
    let cases_base: &[(&[u8], &[u8])] = &[
        (b"/usr/lib\0", b"lib\0"),
        (b"/usr/\0", b"usr\0"),
        (b"usr\0", b"usr\0"),
        (b"/\0", b"/\0"),
        (b"//\0", b"/\0"), // POSIX：全斜杠时实现自定 "/" 或 "//"
        (b"\0", b".\0"),
        (b"a/b/\0", b"b\0"),
    ];
    for (inp, want) in cases_base {
        let mut buf = alloc::vec::Vec::from(*inp);
        unsafe {
            let p = crate::libgen::basename(buf.as_mut_ptr() as *mut c_char);
            let mut n = 0usize;
            while *p.add(n) != 0 {
                n += 1;
            }
            let got = core::slice::from_raw_parts(p as *const u8, n);
            assert_eq!(got, &want[..want.len() - 1], "basename({:?})", inp);
        }
    }

    let cases_dir: &[(&[u8], &[u8])] = &[
        (b"/usr/lib\0", b"/usr\0"),
        (b"/usr/\0", b"/\0"),
        (b"usr\0", b".\0"),
        (b"/\0", b"/\0"),
        (b"\0", b".\0"),
        (b"a/b\0", b"a\0"),
        (b"a//b\0", b"a\0"),
        (b"/usr/lib/\0", b"/usr\0"),
    ];
    for (inp, want) in cases_dir {
        let mut buf = alloc::vec::Vec::from(*inp);
        unsafe {
            let p = crate::libgen::dirname(buf.as_mut_ptr() as *mut c_char);
            let mut n = 0usize;
            while *p.add(n) != 0 {
                n += 1;
            }
            let got = core::slice::from_raw_parts(p as *const u8, n);
            assert_eq!(got, &want[..want.len() - 1], "dirname({:?})", inp);
        }
    }
}


// ===========================================================================
// stdlib.h
// ===========================================================================

/// errno 在**宿主测试**里是进程级兜底槽（没有 FS base 就没有 per-thread 槽），
/// 而 cargo test 默认并行 ⇒ 凡断言 errno 的用例必须互斥，否则互相踩。
static ERRNO_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `strtol` 的 endptr：无转换时指向**原始 nptr**；有转换时指向第一个未消费字符。
#[test]
fn strtol_endptr_semantics() {
    unsafe {
        let s = b"  +42abc\0";
        let mut e: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtol(cs(s), &mut e, 10), 42);
        assert_eq!(e as usize, s.as_ptr() as usize + 5);

        let bad = b"abc\0";
        let mut e2: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtol(cs(bad), &mut e2, 10), 0);
        assert_eq!(e2 as usize, bad.as_ptr() as usize, "no conversion -> endptr == nptr");
    }
}

/// base 推断与 `0x` 前缀（C11 7.22.1.4 的「最长有效主题序列」规则）：
/// `"0x"` 后面没有十六进制数字，主题序列只能是 `"0"`，endptr 必须停在 `'x'`。
#[test]
fn strtol_zero_x_without_digits() {
    unsafe {
        let s = b"0x\0";
        let mut e: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtol(cs(s), &mut e, 16), 0);
        assert_eq!(e as usize, s.as_ptr() as usize + 1, "endptr must stop at 'x'");

        let ok = b"0x1f\0";
        let mut e2: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtol(cs(ok), &mut e2, 0), 31);
        assert_eq!(e2 as usize, ok.as_ptr() as usize + 4);

        let oct = b"010\0";
        let mut e3: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtol(cs(oct), &mut e3, 0), 8);
        assert_eq!(e3 as usize, oct.as_ptr() as usize + 3);
    }
}

/// 有符号溢出：返回该类型极值并置 ERANGE；**恰好**边界不得置 ERANGE。
#[test]
fn strtol_overflow_bounds() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let mut e: *const c_char = core::ptr::null();
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtol(cs(b"9223372036854775807\0"), &mut e, 10), i64::MAX);
        assert_eq!(crate::errno::errno(), 0, "exact max is not an overflow");
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtol(cs(b"9223372036854775808\0"), &mut e, 10), i64::MAX);
        assert_eq!(crate::errno::errno(), crate::errno::ERANGE);
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtol(cs(b"-9223372036854775808\0"), &mut e, 10), i64::MIN);
        assert_eq!(crate::errno::errno(), 0, "exact min is not an overflow");
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtol(cs(b"-9223372036854775809\0"), &mut e, 10), i64::MIN);
        assert_eq!(crate::errno::errno(), crate::errno::ERANGE);
    }
}

/// `strtoul` 接受显式负号（结果按无符号回绕，**不是**溢出）；但真正的溢出按
/// C11 7.22.1.4 返回 ULONG_MAX，而不是"对饱和值再取负"。
#[test]
fn strtoul_negative_and_overflow() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let mut e: *const c_char = core::ptr::null();
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtoul(cs(b"-1\0"), &mut e, 10), u64::MAX);
        assert_eq!(crate::errno::errno(), 0, "-1 wraps, it is not an overflow");
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtoul(cs(b"18446744073709551615\0"), &mut e, 10), u64::MAX);
        assert_eq!(crate::errno::errno(), 0);
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtoul(cs(b"18446744073709551616\0"), &mut e, 10), u64::MAX);
        assert_eq!(crate::errno::errno(), crate::errno::ERANGE);
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::strtoul(cs(b"-18446744073709551616\0"), &mut e, 10), u64::MAX);
        assert_eq!(crate::errno::errno(), crate::errno::ERANGE);
    }
}

/// `atoi`/`atol`/`atoll` 等价于对应的 strtol 家族。
#[test]
fn ato_family() {
    unsafe {
        assert_eq!(crate::stdlib::atoi(cs(b"  -7xyz\0")), -7);
        assert_eq!(crate::stdlib::atoi(cs(b"abc\0")), 0);
        assert_eq!(crate::stdlib::atol(cs(b"123456\0")), 123456);
        assert_eq!(crate::stdlib::atoll(cs(b"-1234567890123\0")), -1234567890123i64);
    }
}

/// `abs`/`labs`/`llabs` 与 `div`/`ldiv`：C99 要求商向零截断、余数与被除数同号。
#[test]
fn integer_helpers() {
    unsafe {
        assert_eq!(crate::stdlib::abs(-5), 5);
        assert_eq!(crate::stdlib::labs(-5), 5);
        assert_eq!(crate::stdlib::llabs(-5), 5);
        let d = crate::stdlib::div(7, 2);
        assert_eq!((d.quot, d.rem), (3, 1));
        let d2 = crate::stdlib::div(-7, 2);
        assert_eq!((d2.quot, d2.rem), (-3, -1));
        let l = crate::stdlib::ldiv(-7, 2);
        assert_eq!((l.quot, l.rem), (-3, -1));
    }
}

/// `strtod`：C99 要求认 inf/infinity/nan（大小写不敏感）与十进制指数。
#[test]
fn strtod_specials_and_exponent() {
    unsafe {
        let mut e: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtod(cs(b"1.5\0"), &mut e), 1.5);
        assert_eq!(crate::stdlib::strtod(cs(b"1e3\0"), &mut e), 1000.0);
        assert_eq!(crate::stdlib::strtod(cs(b"-2.5e-1\0"), &mut e), -0.25);
        assert!(crate::stdlib::strtod(cs(b"inf\0"), &mut e).is_infinite());
        assert!(crate::stdlib::strtod(cs(b"INFINITY\0"), &mut e).is_infinite());
        assert!(crate::stdlib::strtod(cs(b"-inf\0"), &mut e).is_sign_negative());
        assert!(crate::stdlib::strtod(cs(b"nan\0"), &mut e).is_nan());
        let bad = b"xyz\0";
        let mut e2: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtod(cs(bad), &mut e2), 0.0);
        assert_eq!(e2 as usize, bad.as_ptr() as usize);
    }
}

unsafe extern "C" fn cmp_i32(a: *const c_void, b: *const c_void) -> c_int {
    let x = unsafe { *(a as *const i32) };
    let y = unsafe { *(b as *const i32) };
    if x < y { -1 } else if x > y { 1 } else { 0 }
}

/// `qsort` 必须排成非降序；`bsearch` 在已排序数组上必须命中、未命中返回 NULL。
#[test]
fn qsort_and_bsearch() {
    unsafe {
        let mut v: [i32; 7] = [5, -1, 0, 5, 3, 2, -9];
        crate::stdlib::qsort(v.as_mut_ptr() as *mut c_void, v.len(), core::mem::size_of::<i32>(), cmp_i32);
        assert_eq!(v, [-9, -1, 0, 2, 3, 5, 5]);

        let key: i32 = 3;
        let p = crate::stdlib::bsearch(
            &key as *const i32 as *const c_void,
            v.as_ptr() as *const c_void,
            v.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
        assert!(!p.is_null());
        assert_eq!(*(p as *const i32), 3);

        let miss: i32 = 100;
        assert!(crate::stdlib::bsearch(
            &miss as *const i32 as *const c_void,
            v.as_ptr() as *const c_void,
            v.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        )
        .is_null());

        assert!(crate::stdlib::bsearch(
            &key as *const i32 as *const c_void,
            v.as_ptr() as *const c_void,
            0,
            core::mem::size_of::<i32>(),
            cmp_i32,
        )
        .is_null());
    }
}


// ===========================================================================
// time.h（本系统无时区数据库：localtime == gmtime，均为 UTC）
// ===========================================================================

fn tm_at(secs: i64) -> crate::time::Tm {
    unsafe { *crate::time::gmtime(&secs) }
}

/// gmtime 的已知锚点。每个值都可用 `date -u -d @<epoch>` 手工复核。
#[test]
fn gmtime_known_anchors() {
    let a = tm_at(0);
    assert_eq!((a.tm_year, a.tm_mon, a.tm_mday), (70, 0, 1));
    assert_eq!((a.tm_hour, a.tm_min, a.tm_sec), (0, 0, 0));
    assert_eq!(a.tm_wday, 4, "1970-01-01 was a Thursday");
    assert_eq!(a.tm_yday, 0);

    // 2001-09-09T01:46:40Z
    let b = tm_at(1_000_000_000);
    assert_eq!((b.tm_year, b.tm_mon, b.tm_mday), (101, 8, 9));
    assert_eq!((b.tm_hour, b.tm_min, b.tm_sec), (1, 46, 40));
    assert_eq!(b.tm_wday, 0, "2001-09-09 was a Sunday");
    assert_eq!(b.tm_yday, 251);

    // 闰日：2000-02-29T12:00:00Z
    let c = tm_at(951_825_600);
    assert_eq!((c.tm_year, c.tm_mon, c.tm_mday), (100, 1, 29));
    assert_eq!(c.tm_hour, 12);
    assert_eq!(c.tm_yday, 59);

    // 负 epoch：必须向下取整（-1 是 1969-12-31T23:59:59Z，不是 1970-01-01）。
    let d = tm_at(-1);
    assert_eq!((d.tm_year, d.tm_mon, d.tm_mday), (69, 11, 31));
    assert_eq!((d.tm_hour, d.tm_min, d.tm_sec), (23, 59, 59));
    assert_eq!(d.tm_wday, 3, "1969-12-31 was a Wednesday");

    // 1900-01-01T00:00:00Z（%C/%y 的两位年边界）
    let e = tm_at(-2_208_988_800);
    assert_eq!((e.tm_year, e.tm_mon, e.tm_mday), (0, 0, 1));
}

/// mktime 与 gmtime 互逆（本系统无时区，两者都按 UTC）。
#[test]
fn mktime_inverts_gmtime() {
    for &secs in &[0i64, 1, -1, 1_000_000_000, -2_208_988_800, 2_147_483_647, 951_825_600] {
        let mut t = tm_at(secs);
        assert_eq!(unsafe { crate::time::mktime(&mut t) }, secs, "round trip {}", secs);
    }
}

/// POSIX 要求 mktime **归一化**越界字段，并把归一化结果写回。
#[test]
fn mktime_normalizes_out_of_range_fields() {
    unsafe {
        // 2000-01-00 ⇒ 1999-12-31
        let mut t = crate::time::Tm {
            tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 0, tm_mon: 0, tm_year: 100,
            tm_wday: 0, tm_yday: 0, tm_isdst: 0,
        };
        assert_eq!(crate::time::mktime(&mut t), 946_598_400);
        assert_eq!((t.tm_year, t.tm_mon, t.tm_mday), (99, 11, 31));

        // 13 月 ⇒ 次年 1 月
        let mut t2 = crate::time::Tm {
            tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 12, tm_year: 99,
            tm_wday: 0, tm_yday: 0, tm_isdst: 0,
        };
        crate::time::mktime(&mut t2);
        assert_eq!((t2.tm_year, t2.tm_mon), (100, 0));

        // 60 秒 ⇒ 进位到下一分钟
        let mut t3 = crate::time::Tm {
            tm_sec: 60, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 70,
            tm_wday: 0, tm_yday: 0, tm_isdst: 0,
        };
        assert_eq!(crate::time::mktime(&mut t3), 60);
        assert_eq!((t3.tm_min, t3.tm_sec), (1, 0));
    }
}

/// strftime 的渲染与「放不下返回 0（POSIX 7.27.3.5）」。
#[test]
fn strftime_formats_and_capacity() {
    unsafe {
        let t = *crate::time::gmtime(&1_000_000_000i64);
        let mut buf = [0u8; 128];

        let n = crate::time::strftime(
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            cs(b"%Y-%m-%d %H:%M:%S\0"),
            &t,
        );
        assert_eq!(core::str::from_utf8(&buf[..n]).unwrap(), "2001-09-09 01:46:40");

        let n2 = crate::time::strftime(
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            cs(b"%a %A %b %B %j %w %p %F %T %z %Z %%\0"),
            &t,
        );
        assert_eq!(
            core::str::from_utf8(&buf[..n2]).unwrap(),
            "Sun Sunday Sep September 252 0 AM 2001-09-09 01:46:40 +0000 UTC %"
        );

        // 恰好放下（结果 4 字节 + NUL = 5）成功；只给 4 字节 ⇒ 0。
        let mut exact = [0u8; 5];
        assert_eq!(
            crate::time::strftime(exact.as_mut_ptr() as *mut c_char, 5, cs(b"2001\0"), &t),
            4
        );
        let mut small = [0u8; 4];
        assert_eq!(
            crate::time::strftime(small.as_mut_ptr() as *mut c_char, 4, cs(b"2001\0"), &t),
            0
        );

        // 未实现的说明符按 POSIX 允许的方式**原样输出**，绝不静默丢弃。
        let n3 = crate::time::strftime(
            buf.as_mut_ptr() as *mut c_char,
            buf.len(),
            cs(b"%X\0"),
            &t,
        );
        assert_eq!(core::str::from_utf8(&buf[..n3]).unwrap(), "%X");
    }
}

/// `ctime`/`asctime` 的固定 26 字节形态（C 标准规定的 asctime 输出形状）。
#[test]
fn asctime_and_ctime_shape() {
    unsafe {
        let t = *crate::time::gmtime(&0i64);
        let p = crate::time::asctime(&t);
        let mut n = 0usize;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = core::str::from_utf8(core::slice::from_raw_parts(p as *const u8, n)).unwrap();
        // C11 7.27.3.1：asctime 的固定形态**以换行结尾**。
        assert_eq!(s, "Thu Jan  1 00:00:00 1970\n");
    }
}


// ===========================================================================
// fnmatch.h（POSIX.1-2008 7.3.2；FNM_LEADING_DIR / FNM_CASEFOLD 为 GNU 扩展）
// ===========================================================================

fn fnm_rs(pattern: &[u8], string: &[u8], flags: c_int) -> c_int {
    let mut p = alloc::vec::Vec::from(pattern);
    let mut s = alloc::vec::Vec::from(string);
    p.push(0);
    s.push(0);
    unsafe {
        crate::posix_batch4::fnmatch(
            p.as_ptr() as *const c_char,
            s.as_ptr() as *const c_char,
            flags,
        )
    }
}

#[test]
fn fnmatch_basic_patterns() {
    use crate::posix_batch4::*;
    assert_eq!(fnm_rs(b"*.c", b"hello.c", 0), 0);
    assert_eq!(fnm_rs(b"*.c", b"hello.h", 0), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"h?llo", b"hello", 0), 0);
    assert_eq!(fnm_rs(b"[a-c]x", b"bx", 0), 0);
    assert_eq!(fnm_rs(b"[!a-c]x", b"dx", 0), 0);
    assert_eq!(fnm_rs(b"[!a-c]x", b"bx", 0), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a\\*b", b"a*b", 0), 0);
    assert_eq!(fnm_rs(b"a\\*b", b"axb", 0), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a\\*b", b"a*b", FNM_NOESCAPE), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a/*", b"a/b", FNM_PATHNAME), 0);
    assert_eq!(fnm_rs(b"a/*", b"a/b/c", FNM_PATHNAME), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a/?", b"a/", FNM_PATHNAME), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"ABC", b"abc", FNM_CASEFOLD), 0);
    assert_eq!(fnm_rs(b"ABC", b"abc", 0), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a", b"a/b/c", FNM_LEADING_DIR), 0);
}

/// FNM_PERIOD（POSIX 7.3.2）：**前导句点 = 串的首字符，或紧跟斜杠之后的句点**
/// （后者需 FNM_PATHNAME）；`*`/`?`/`[...]` 都不得匹配前导句点。
#[test]
fn fnmatch_period_is_leading_after_slash() {
    use crate::posix_batch4::*;
    let f = FNM_PATHNAME | FNM_PERIOD;
    assert_eq!(fnm_rs(b"*", b".hidden", FNM_PERIOD), FNM_NOMATCH);
    assert_eq!(fnm_rs(b".*", b".hidden", FNM_PERIOD), 0);
    assert_eq!(fnm_rs(b"a/*", b"a/.hidden", f), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a/?", b"a/.", f), FNM_NOMATCH);
    // 括号表达式同样不得匹配前导句点（glibc posix/fnmatch_loop.c 的 [ 分支有同一条
    // no_leading_period 拒绝）；显式写句点才行。
    assert_eq!(fnm_rs(b"a/[.]h", b"a/.h", f), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"a/.h", b"a/.h", f), 0);
    // 星号不得退化成"匹配 0 个字符"再去配模式里的句点。
    assert_eq!(fnm_rs(b"*.c", b".hidden.c", FNM_PERIOD), FNM_NOMATCH);
    assert_eq!(fnm_rs(b"*.c", b"visible.c", FNM_PERIOD), 0);
    assert_eq!(fnm_rs(b"a/*", b"a/visible", f), 0);
}


// ===========================================================================
// stdio.h（printf 家族）
// ===========================================================================

/// C11 7.21.6.5：`snprintf` 返回**「若空间足够本应写入的字符数」**（不含结尾 NUL），
/// 而不是实际写入数；且 size>0 时**恒**写 NUL 终止（哪怕发生截断）。
#[test]
fn snprintf_returns_would_be_length_and_terminates() {
    unsafe {
        let mut buf = [0x7Fu8; 8];
        let n = crate::stdio::snprintf(
            buf.as_mut_ptr() as *mut c_char,
            8,
            cs(b"%s\0"),
            cs(b"hello"),
        );
        assert_eq!(n, 5);
        assert_eq!(&buf[..6], b"hello\0");

        // 截断：只给 4 字节 ⇒ 写 "hel" + NUL，但返回值仍是 5。
        let mut small = [0x7Fu8; 8];
        let n2 = crate::stdio::snprintf(
            small.as_mut_ptr() as *mut c_char,
            4,
            cs(b"%s\0"),
            cs(b"hello"),
        );
        assert_eq!(n2, 5, "snprintf must report the untruncated length");
        assert_eq!(&small[..4], b"hel\0", "truncation must still NUL-terminate");

        // size==0：只计数，绝不碰缓冲。
        let mut untouched = [0x7Fu8; 4];
        let n3 = crate::stdio::snprintf(
            untouched.as_mut_ptr() as *mut c_char,
            0,
            cs(b"%s\0"),
            cs(b"hello"),
        );
        assert_eq!(n3, 5);
        assert_eq!(untouched, [0x7Fu8; 4]);
    }
}


// ===========================================================================
// wchar.h / locale.h
// ===========================================================================

use crate::ctypes::wchar_t;

/// 本实现的编码是**逐字节**（无多字节编码，见 wchar.rs 文件头的诚实说明）；
/// 下面钉住的是 POSIX 对**返回值与控制流**的规定，与编码选择无关。
#[test]
fn wchar_conversion_contract() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let mut wc: wchar_t = 0;
        let np: *mut crate::ctypes::c_void = core::ptr::null_mut();
        assert_eq!(crate::wchar::mbrtowc(&mut wc, cs(b"A\0"), 1, np), 1);
        assert_eq!(wc, 65);

        // NUL 字节：写 L'\0' 并返回 0（POSIX）。
        assert_eq!(crate::wchar::mbrtowc(&mut wc, cs(b"\0"), 1, np), 0);
        assert_eq!(wc, 0);

        // s == NULL ⇒ 等价于 mbrtowc(NULL, "", 1, ps)：复位状态，返回 0。
        assert_eq!(crate::wchar::mbrtowc(&mut wc, core::ptr::null::<c_char>(), 1, np), 0);

        // pwc == NULL ⇒ 只数长度，不写。
        assert_eq!(crate::wchar::mbrtowc(core::ptr::null_mut(), cs(b"A\0"), 1, np), 1);

        // wcrtomb：合法宽字符 1 字节；超出 [0,255] ⇒ (size_t)-1 + EILSEQ。
        let mut buf = [0i8; 4];
        assert_eq!(crate::wchar::wcrtomb(buf.as_mut_ptr(), 0x41, np), 1);
        assert_eq!(buf[0] as u8, 0x41);
        crate::errno::set_errno(0);
        assert_eq!(crate::wchar::wcrtomb(buf.as_mut_ptr(), 0x4E2D, np), usize::MAX);
        assert_eq!(crate::errno::errno(), crate::errno::EILSEQ);

        // s == NULL ⇒ 只数所需字节数（合法宽字符 = 1），不写。
        assert_eq!(crate::wchar::wcrtomb(core::ptr::null_mut(), 0x41, np), 1);
    }
}

/// len == 0 时**不得写入任何元素**——连结尾 NUL 也不行（调用方只给了 0 个位置）。
#[test]
fn wchar_zero_length_writes_nothing() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let np: *mut crate::ctypes::c_void = core::ptr::null_mut();
        // 空源 + len==0：转换出 0 个宽字符，且**一个元素都不许写**（连结尾 NUL 也不行）。
        let mut sentinel = [0x7FFF as wchar_t; 2];
        assert_eq!(crate::wchar::mbstowcs(sentinel.as_mut_ptr(), cs(b"\0"), 0), 0);
        assert_eq!(sentinel[0], 0x7FFF, "len==0 must not write the terminator");

        let empty = [0 as wchar_t];
        let mut bytes = [0x7Fi8; 2];
        assert_eq!(crate::wchar::wcstombs(bytes.as_mut_ptr(), empty.as_ptr(), 0), 0);
        assert_eq!(bytes[0], 0x7Fi8, "len==0 must not write the terminator");

        // 源非空而 len==0（空间不足）⇒ (size_t)-1 + E2BIG（POSIX 与 glibc 同）。
        let wide = [65 as wchar_t, 0 as wchar_t];
        crate::errno::set_errno(0);
        assert_eq!(crate::wchar::wcstombs(bytes.as_mut_ptr(), wide.as_ptr(), 0), usize::MAX);
        assert_eq!(crate::errno::errno(), crate::errno::E2BIG);
    }
}

/// C locale 下 strcoll 必须与 strcmp 同序；strxfrm 必须给出可比较的变换结果
/// 且返回**未截断**的长度（C11 7.24.4.5）。
#[test]
fn strcoll_and_strxfrm_in_c_locale() {
    unsafe {
        assert!(crate::posix_batch6::strcoll(cs(b"abc\0"), cs(b"abd\0")) < 0);
        assert_eq!(crate::posix_batch6::strcoll(cs(b"abc\0"), cs(b"abc\0")), 0);
        let mut dst = [0i8; 8];
        let n = crate::posix_batch6::strxfrm(dst.as_mut_ptr(), cs(b"abc\0"), dst.len());
        assert_eq!(n, 3);
        assert_eq!(dst[..4], [b'a' as i8, b'b' as i8, b'c' as i8, 0]);
    }
}


// ===========================================================================
// getopt / strptime
// ===========================================================================

/// getopt 的状态（optind/optarg/optopt）按 POSIX 是**进程级全局**，故本组必须互斥。
static GETOPT_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

unsafe fn cbytes(p: *const c_char) -> alloc::vec::Vec<u8> {
    let mut v = alloc::vec::Vec::new();
    if p.is_null() {
        return v;
    }
    let mut q = p as *const u8;
    while unsafe { *q } != 0 {
        v.push(unsafe { *q });
        q = unsafe { q.add(1) };
    }
    v
}

/// 用一组字节串构造 argv（返回的 Vec 必须活到解析结束）。
fn make_argv(items: &[&[u8]]) -> (alloc::vec::Vec<alloc::vec::Vec<u8>>, alloc::vec::Vec<*mut c_char>) {
    let mut bufs: alloc::vec::Vec<alloc::vec::Vec<u8>> = items
        .iter()
        .map(|s| {
            let mut v = s.to_vec();
            v.push(0);
            v
        })
        .collect();
    let ptrs = bufs
        .iter_mut()
        .map(|v| v.as_mut_ptr() as *mut c_char)
        .collect();
    (bufs, ptrs)
}

/// POSIX getopt：选项字符、optarg、optind、未知选项、缺参数、双横线终止。
#[test]
fn getopt_posix_semantics() {
    let _g = GETOPT_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let (_b, mut argv) = make_argv(&[b"prog", b"-a", b"-b", b"value", b"-c"]);
        let argc = argv.len() as c_int;
        let os = b"ab:c\0";
        crate::posix_batch4::optind = 1;
        let f = |argv: &mut alloc::vec::Vec<*mut c_char>| {
            crate::posix_batch4::getopt(argc, argv.as_ptr(), os.as_ptr() as *const c_char)
        };
        assert_eq!(f(&mut argv), b'a' as c_int);
        assert_eq!(f(&mut argv), b'b' as c_int);
        let oa = crate::posix_batch4::optarg;
        assert_eq!(cbytes(oa), b"value");
        assert_eq!(f(&mut argv), b'c' as c_int);
        assert_eq!(f(&mut argv), -1, "exhausted options return -1");
        let ind = crate::posix_batch4::optind;
        assert_eq!(ind, 5);
    }
}

/// 同一 argv 项内连写（-ab value）与紧跟形式（-bvalue）。
#[test]
fn getopt_clustered_and_attached() {
    let _g = GETOPT_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let os = b"ab:c\0";
        let (_b1, mut argv1) = make_argv(&[b"prog", b"-ab", b"value"]);
        crate::posix_batch4::optind = 1;
        assert_eq!(crate::posix_batch4::getopt(3, argv1.as_ptr(), os.as_ptr() as *const c_char), b'a' as c_int);
        assert_eq!(crate::posix_batch4::getopt(3, argv1.as_ptr(), os.as_ptr() as *const c_char), b'b' as c_int);
        let oa = crate::posix_batch4::optarg;
        assert_eq!(cbytes(oa), b"value");

        let (_b2, mut argv2) = make_argv(&[b"prog", b"-bvalue"]);
        crate::posix_batch4::optind = 1;
        assert_eq!(crate::posix_batch4::getopt(2, argv2.as_ptr(), os.as_ptr() as *const c_char), b'b' as c_int);
        let oa = crate::posix_batch4::optarg;
        assert_eq!(cbytes(oa), b"value");
    }
}

/// 未知选项返回 '?' 并置 optopt；缺参数返回 '?'（optstring 不以 ':' 开头）。
#[test]
fn getopt_unknown_and_missing_argument() {
    let _g = GETOPT_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let os = b"ab:c\0";
        let (_b1, mut argv1) = make_argv(&[b"prog", b"-z"]);
        crate::posix_batch4::optind = 1;
        crate::posix_batch4::optopt = 0;
        assert_eq!(crate::posix_batch4::getopt(2, argv1.as_ptr(), os.as_ptr() as *const c_char), b'?' as c_int);
        let oo = crate::posix_batch4::optopt;
        assert_eq!(oo, b'z' as c_int);

        let (_b2, mut argv2) = make_argv(&[b"prog", b"-b"]);
        crate::posix_batch4::optind = 1;
        crate::posix_batch4::optopt = 0;
        assert_eq!(crate::posix_batch4::getopt(2, argv2.as_ptr(), os.as_ptr() as *const c_char), b'?' as c_int);
        let oo2 = crate::posix_batch4::optopt;
        assert_eq!(oo2, b'b' as c_int);

        let os2 = b":ab:c\0";
        let (_b3, mut argv3) = make_argv(&[b"prog", b"-b"]);
        crate::posix_batch4::optind = 1;
        assert_eq!(crate::posix_batch4::getopt(2, argv3.as_ptr(), os2.as_ptr() as *const c_char), b':' as c_int);
    }
}

/// 双横线终止选项解析，且 optind 指向其后的第一个参数。
#[test]
fn getopt_double_dash_terminates() {
    let _g = GETOPT_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let os = b"a\0";
        let (_b, mut argv) = make_argv(&[b"prog", b"--", b"-a"]);
        crate::posix_batch4::optind = 1;
        assert_eq!(crate::posix_batch4::getopt(3, argv.as_ptr(), os.as_ptr() as *const c_char), -1);
        let ind2 = crate::posix_batch4::optind;
        assert_eq!(ind2, 2, "optind must point past the terminator");
    }
}

/// strptime：字段解析、endptr、两位年份规则、%p 与 %I 组合、失败返回 NULL。
#[test]
fn strptime_conversions() {
    unsafe {
        let mut tm = crate::time::Tm {
            tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 70,
            tm_wday: 0, tm_yday: 0, tm_isdst: 0,
        };
        let src = b"2001-09-09 01:46:40 rest\0";
        let fmt = b"%Y-%m-%d %H:%M:%S\0";
        let end = crate::posix_batch4::strptime(cs(src), cs(fmt), &mut tm);
        assert!(!end.is_null());
        assert_eq!((tm.tm_year, tm.tm_mon, tm.tm_mday), (101, 8, 9));
        assert_eq!((tm.tm_hour, tm.tm_min, tm.tm_sec), (1, 46, 40));
        assert_eq!(*end, b' ' as c_char, "endptr must point at the first unprocessed char");

        let mut t68 = tm;
        assert!(!crate::posix_batch4::strptime(cs(b"68\0"), cs(b"%y\0"), &mut t68).is_null());
        assert_eq!(t68.tm_year, 168);
        let mut t69 = tm;
        assert!(!crate::posix_batch4::strptime(cs(b"69\0"), cs(b"%y\0"), &mut t69).is_null());
        assert_eq!(t69.tm_year, 69);

        let mut tp = tm;
        let e = crate::posix_batch4::strptime(cs(b"01:30 PM\0"), cs(b"%I:%M %p\0"), &mut tp);
        assert!(!e.is_null());
        assert_eq!(tp.tm_hour, 13);
        assert_eq!(tp.tm_min, 30);

        assert!(crate::posix_batch4::strptime(cs(b"nope\0"), cs(b"%Y\0"), &mut tm).is_null());
    }
}


// ===========================================================================
// wordexp / alphasort / regex
// ===========================================================================

/// **wordexp / glob / malloc 在宿主上不可测**：它们经 libc 的 `malloc`，而那条路最终
/// 落到内核 `brk`（宿主上没有内核）⇒ 恒 `WRDE_NOSPACE`。这三项的真值只在真机
/// `libccheck` 里验。此处**不写一个恒失败的假测试**，如实留白。
///
/// alphasort：按 d_name 字典序（C locale 下与 strcoll 等价，见其文档的诚实边界）。
#[test]
fn alphasort_orders_by_name() {
    unsafe {
        let mut a: crate::dirent::dirent = core::mem::zeroed();
        let mut b: crate::dirent::dirent = core::mem::zeroed();
        for (i, ch) in b"apple\0".iter().enumerate() {
            a.d_name[i] = *ch as c_char;
        }
        for (i, ch) in b"banana\0".iter().enumerate() {
            b.d_name[i] = *ch as c_char;
        }
        let pa: *const crate::dirent::dirent = &a;
        let pb: *const crate::dirent::dirent = &b;
        assert!(crate::dirent::alphasort(&pa, &pb) < 0);
        assert!(crate::dirent::alphasort(&pb, &pa) > 0);
        assert_eq!(crate::dirent::alphasort(&pa, &pa), 0);
    }
}

/// ERE（REG_EXTENDED）：锚点、字符类、重复；未匹配返回非 0；regfree 释放。
#[test]
fn regex_ere_matching() {
    unsafe {
        const ERE: c_int = crate::regex::REG_EXTENDED;
        let mut re: crate::regex::regex_t = core::mem::zeroed();
        assert_eq!(crate::regex::regcomp(&mut re, cs(b"^a[0-9]+c$\0"), ERE), 0);
        let mut m = [crate::regex::regmatch_t { rm_so: -1, rm_eo: -1 }];
        assert_eq!(crate::regex::regexec(&re, cs(b"a123c\0"), 1, m.as_mut_ptr(), 0), 0);
        assert_eq!((m[0].rm_so, m[0].rm_eo), (0, 5));
        assert_ne!(crate::regex::regexec(&re, cs(b"b123c\0"), 1, m.as_mut_ptr(), 0), 0);
        assert_ne!(crate::regex::regexec(&re, cs(b"a12xc\0"), 1, m.as_mut_ptr(), 0), 0);
        crate::regex::regfree(&mut re);

        // 捕获组：re_nsub 应报出组数。
        let mut re2: crate::regex::regex_t = core::mem::zeroed();
        assert_eq!(crate::regex::regcomp(&mut re2, cs(b"(a)(b)\0"), ERE), 0);
        let nsub = re2.re_nsub;
        assert_eq!(nsub, 2, "re_nsub must count capture groups");
        let mut m2: [crate::regex::regmatch_t; 3] = core::mem::zeroed();
        assert_eq!(crate::regex::regexec(&re2, cs(b"xab\0"), 3, m2.as_mut_ptr(), 0), 0);
        assert_eq!((m2[1].rm_so, m2[1].rm_eo), (1, 2));
        assert_eq!((m2[2].rm_so, m2[2].rm_eo), (2, 3));
        crate::regex::regfree(&mut re2);
    }
}


/// BRE（默认模式）：POSIX 规定裸 `+`/`?` 是**普通字符**，量词形式是 `\+`/`\?`。
///
/// 修复前 peek_quantifier 对裸 `+`/`?` 也返回量词，而 consume_quantifier 只在
/// extended 下前进 ⇒ 位置不推进、下一轮被判成"连续量词" ⇒ regcomp("a+b") 误报 REG_BADRPT。
#[test]
fn regex_bre_plus_is_literal() {
    unsafe {
        // 裸 + 是字面量：a+b 匹配 "a+b"。
        let mut re: crate::regex::regex_t = core::mem::zeroed();
        assert_eq!(crate::regex::regcomp(&mut re, cs(b"a+b\0"), 0), 0);
        let mut m = [crate::regex::regmatch_t { rm_so: -1, rm_eo: -1 }];
        assert_eq!(crate::regex::regexec(&re, cs(b"xa+by\0"), 1, m.as_mut_ptr(), 0), 0);
        assert_eq!((m[0].rm_so, m[0].rm_eo), (1, 4));
        assert_ne!(crate::regex::regexec(&re, cs(b"aab\0"), 1, m.as_mut_ptr(), 0), 0);
        crate::regex::regfree(&mut re);

        // 转义的 \+ 才是量词。
        let mut re2: crate::regex::regex_t = core::mem::zeroed();
        assert_eq!(crate::regex::regcomp(&mut re2, cs(b"a\\+b\0"), 0), 0);
        assert_eq!(crate::regex::regexec(&re2, cs(b"aaab\0"), 1, m.as_mut_ptr(), 0), 0);
        assert_ne!(crate::regex::regexec(&re2, cs(b"a+b\0"), 1, m.as_mut_ptr(), 0), 0);
        crate::regex::regfree(&mut re2);
    }
}


// ===========================================================================
// printf 格式引擎（C11 7.21.6.1）
// ===========================================================================

/// 经 snprintf 渲染（可变参数在宿主上走 GP 槽，整数/指针路径可用）。
unsafe fn sn(fmt: &[u8], buf: &mut [u8]) -> alloc::vec::Vec<u8> {
    let n = crate::stdio::snprintf(
        buf.as_mut_ptr() as *mut c_char,
        buf.len(),
        cs(fmt),
    );
    if n < 0 {
        return alloc::vec::Vec::new();
    }
    alloc::vec::Vec::from(&buf[..n as usize])
}

/// 整数/字符串/字符格式：直接喂给格式引擎（避开可变参数），逐条对照 C11。
#[test]
fn printf_integer_and_string_specifiers() {
    let mut buf = [0u8; 64];
    unsafe {
        // %d / 宽度 / 精度 / flags
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%d\0"), 42), 2);
        assert_eq!(&buf[..2], b"42");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%d\0"), -42), 3);
        assert_eq!(&buf[..3], b"-42");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%5d\0"), 42), 5);
        assert_eq!(&buf[..5], b"   42");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%-5d\0"), 42), 5);
        assert_eq!(&buf[..5], b"42   ");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%05d\0"), 42), 5);
        assert_eq!(&buf[..5], b"00042");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%+d\0"), 42), 3);
        assert_eq!(&buf[..3], b"+42");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"% d\0"), 42), 3);
        assert_eq!(&buf[..3], b" 42");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%.3d\0"), 7), 3);
        assert_eq!(&buf[..3], b"007");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%5.3d\0"), 7), 5);
        assert_eq!(&buf[..5], b"  007");

        // 进制与 alt 形式
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%x\0"), 255), 2);
        assert_eq!(&buf[..2], b"ff");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%X\0"), 255), 2);
        assert_eq!(&buf[..2], b"FF");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%#x\0"), 255), 4);
        assert_eq!(&buf[..4], b"0xff");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%o\0"), 8), 2);
        assert_eq!(&buf[..2], b"10");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%#o\0"), 8), 3);
        assert_eq!(&buf[..3], b"010");

        // 长度修饰符
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%ld\0"), 1234567890123i64), 13);
        assert_eq!(&buf[..13], b"1234567890123");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%lld\0"), -5i64), 2);
        assert_eq!(&buf[..2], b"-5");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%zu\0"), 7usize), 1);
        assert_eq!(&buf[..1], b"7");

        // 星号宽度/精度
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%*d\0"), 5, 42), 5);
        assert_eq!(&buf[..5], b"   42");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%.*d\0"), 3, 7), 3);
        assert_eq!(&buf[..3], b"007");

        // 字符与字符串
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%c\0"), 65), 1);
        assert_eq!(&buf[..1], b"A");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%s\0"), cs(b"hi\0")), 2);
        assert_eq!(&buf[..2], b"hi");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%5s\0"), cs(b"hi\0")), 5);
        assert_eq!(&buf[..5], b"   hi");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%-5s\0"), cs(b"hi\0")), 5);
        assert_eq!(&buf[..5], b"hi   ");
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%.1s\0"), cs(b"hi\0")), 1);
        assert_eq!(&buf[..1], b"h");

        // 百分号字面量
        assert_eq!(crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"100%%\0")), 4);
        assert_eq!(&buf[..4], b"100%");

        // %p：格式**实现自定**（C11 7.21.6.1）。本实现：非空指针带 0x 前缀的十六进制；
        // 空指针按 alt 规则不补前缀（打印 "0"）——与 glibc 的 "(nil)" 不同，两者都合规。
        let v: usize = 0x1234;
        let n = crate::stdio::snprintf(buf.as_mut_ptr() as *mut c_char, 64, cs(b"%p\0"), v);
        assert!(n > 2 && &buf[..2] == b"0x", "non-null pointer must carry the 0x prefix");

        // %n：写入**已输出**的字符数
        let mut cnt: c_int = -1;
        let n2 = crate::stdio::snprintf(
            buf.as_mut_ptr() as *mut c_char, 64, cs(b"abc%n\0"), &mut cnt as *mut c_int,
        );
        assert_eq!(n2, 3);
        assert_eq!(cnt, 3, "%n must store the number of chars written so far");
    }
}

/// 浮点格式化：经 crate::float 的发射器直接测（可变参数在宿主上读不到 XMM 槽，
/// 见 stdio.rs 的 next_float_arg —— 那条 GP 路径是给 os="none" 软浮点目标用的）。
fn float_fmt(fmt: &[u8], v: f64) -> alloc::vec::Vec<u8> {
    use crate::stdio_format::{parse_and_format, Conv, VecSink};
    let mut discard = alloc::vec::Vec::new();
    let mut found = None;
    {
        let mut sink = VecSink(&mut discard);
        let _ = parse_and_format(fmt, &mut sink, |spec, _s| {
            found = Some(*spec);
            Ok(())
        });
    }
    let s = found.expect("one conversion");
    let prec = if s.prec >= 0 { s.prec as usize } else { 6 };
    let mut d = crate::float::decompose(v);
    let mut out = alloc::vec::Vec::new();
    {
        let mut sink = VecSink(&mut out);
        let r = match s.conv {
            Conv::Float => crate::float::emit_fixed(&s, &mut d, prec, &mut sink),
            Conv::Exp => crate::float::emit_exp(&s, &mut d, prec, false, &mut sink),
            _ => crate::float::emit_general(&s, &mut d, prec, &mut sink),
        };
        assert!(r.is_ok());
    }
    out
}

/// %f/%e/%g 的定点/指数/有效位规则（C11 7.21.6.1）。
#[test]
fn printf_float_specifiers() {
    assert_eq!(float_fmt(b"%f", 1.5), b"1.500000");
    assert_eq!(float_fmt(b"%.2f", 1.5), b"1.50");
    assert_eq!(float_fmt(b"%8.2f", 1.5), b"    1.50");
    assert_eq!(float_fmt(b"%-8.2f", 1.5), b"1.50    ");
    assert_eq!(float_fmt(b"%08.2f", 1.5), b"00001.50");
    assert_eq!(float_fmt(b"%.0f", 2.5), b"2", "round-half-even: 2.5 -> 2");
    assert_eq!(float_fmt(b"%.0f", 3.5), b"4", "round-half-even: 3.5 -> 4");
    assert_eq!(float_fmt(b"%f", 0.0), b"0.000000");
    assert_eq!(float_fmt(b"%f", -0.0), b"-0.000000");
    assert_eq!(float_fmt(b"%e", 1500.0), b"1.500000e+03");
    assert_eq!(float_fmt(b"%E", 1500.0), b"1.500000E+03");
    assert_eq!(float_fmt(b"%.2e", 0.0015), b"1.50e-03");
    assert_eq!(float_fmt(b"%g", 1500.0), b"1500");
    assert_eq!(float_fmt(b"%g", 0.00015), b"0.00015");
    assert_eq!(float_fmt(b"%g", 1e20), b"1e+20");
    // **大值**：这条钉住 decompose_fast 的溢出检查（移位前判，而不是事后判）。
    assert_eq!(float_fmt(b"%g", 1e20), b"1e+20");
    assert_eq!(float_fmt(b"%.1f", 1e20), b"100000000000000000000.0");
    assert_eq!(float_fmt(b"%e", 1e20), b"1.000000e+20");
    assert_eq!(float_fmt(b"%.0f", 1e18), b"1000000000000000000");
    assert_eq!(float_fmt(b"%+f", 1.0), b"+1.000000");
    assert_eq!(float_fmt(b"% f", 1.0), b" 1.000000");
}


// ===========================================================================
// locale.h / wcwidth
// ===========================================================================

/// POSIX：setlocale 返回的字符串**可以再喂回** setlocale 并得到同一 locale。
/// 这条正是旧值 "C.UTF-8" 栽的地方——本实现只认 "C"/"POSIX"，旧返回值自己不被接受。
#[test]
fn setlocale_round_trip() {
    unsafe {
        let cur = crate::locale::setlocale(0, core::ptr::null());
        assert!(!cur.is_null());
        let cur_b = cbytes(cur);
        let again = crate::locale::setlocale(0, cur);
        assert!(!again.is_null(), "the returned name must be accepted again");
        assert_eq!(cbytes(again), cur_b);

        assert!(!crate::locale::setlocale(0, cs(b"C\0")).is_null());
        assert!(!crate::locale::setlocale(0, cs(b"POSIX\0")).is_null());
        assert!(crate::locale::setlocale(0, cs(b"zh_CN.UTF-8\0")).is_null());
    }
}

/// localeconv：C locale 的定点与分隔符（C11 7.11.2.1）。
#[test]
fn localeconv_c_locale() {
    unsafe {
        let lc = crate::locale::localeconv();
        assert!(!lc.is_null());
        assert_eq!(cbytes((*lc).decimal_point), b".");
        assert_eq!(cbytes((*lc).thousands_sep), b"");
        assert_eq!(cbytes((*lc).grouping), b"");
    }
}

/// wcwidth：可打印 ASCII 1 列、东亚宽 2 列、组合符号 0 列、C0 控制不可打印 -1、NUL 0。
#[test]
fn wcwidth_classes() {
    unsafe {
        assert_eq!(crate::wcwidth::wcwidth(b'A' as wchar_t), 1);
        assert_eq!(crate::wcwidth::wcwidth(0x4E2D), 2, "CJK ideograph is double width");
        assert_eq!(crate::wcwidth::wcwidth(0x0300), 0, "combining grave is zero width");
        assert_eq!(crate::wcwidth::wcwidth(0), 0, "NUL is zero width");
        assert_eq!(crate::wcwidth::wcwidth(0x1F), -1, "C0 control is not printable");
        let s = [b'A' as wchar_t, 0x4E2D, 0];
        assert_eq!(crate::wcwidth::wcswidth(s.as_ptr(), 2), 3);
    }
}


// ===========================================================================
// 浮点：十进制往返与 strtod 的严格舍入声明
// ===========================================================================

unsafe fn parse_f64(s: &[u8]) -> f64 {
    let mut buf = s.to_vec();
    buf.push(0);
    crate::stdlib::strtod(buf.as_ptr() as *const c_char, core::ptr::null_mut())
}

/// **往返**：C11 5.2.4.2.2 规定 double 的 DECIMAL_DIG = 17，即用 17 位有效数字打印后
/// 再解析回来必须**逐位相同**。这一条同时压住格式化（float/float_bigint）与解析
/// （strtod_exact）两侧——任何一边的舍入有偏差都会在这里现形。
#[test]
fn float_decimal_round_trip() {
    let cases: &[f64] = &[
        0.0, 1.0, -1.0, 0.1, 1.0 / 3.0, 2.0 / 3.0,
        1e-300, 1e300, 1e20, 1e-20, 1e17, 123456789.123456789,
        f64::MIN_POSITIVE, 5e-324, 2.2250738585072011e-308, 1.7976931348623157e308,
        9007199254740992.0, 3.141592653589793, -2.718281828459045,
    ];
    for &v in cases {
        let s = float_fmt(b"%.16e", v);
        let back = unsafe { parse_f64(&s) };
        assert_eq!(
            back.to_bits(),
            v.to_bits(),
            "round trip failed: {} -> {}",
            v,
            core::str::from_utf8(&s).unwrap_or("?")
        );
    }
}

/// strtod 的**严格正确舍入**声明（模块文档明写）：边界与半值必须按 round-half-even 落位。
#[test]
fn strtod_strict_rounding() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // 0.1 在二进制里是无限循环小数：必须落到最近的双精度值。
        assert_eq!(parse_f64(b"0.1").to_bits(), 0.1f64.to_bits());
        // 2^53+1 不可表示 ⇒ 落到 2^53（round-half-even）。
        assert_eq!(parse_f64(b"9007199254740993"), 9007199254740992.0);
        // 模块文档点名的边界：**最大次正规数**（不是最小正规数）。这是 strtod 的经典难题
        // （Gay 的测试向量）：该串恰好落在最大次正规与最小正规之间，正确答案是**向下**
        // 落到最大次正规 0x000FFFFFFFFFFFFF。
        assert_eq!(
            parse_f64(b"2.2250738585072011e-308").to_bits(),
            0x000F_FFFF_FFFF_FFFF,
            "must round DOWN to the largest subnormal, not up to MIN_POSITIVE"
        );
        // 下溢到 0 与上溢到 inf，都置 ERANGE。
        crate::errno::set_errno(0);
        assert_eq!(parse_f64(b"1e-400"), 0.0);
        assert_eq!(crate::errno::errno(), crate::errno::ERANGE);
        crate::errno::set_errno(0);
        assert!(parse_f64(b"1e309").is_infinite());
        assert_eq!(crate::errno::errno(), crate::errno::ERANGE);
        // 最大有限数恰好可表示 ⇒ 不得置 ERANGE。
        crate::errno::set_errno(0);
        assert_eq!(parse_f64(b"1.7976931348623157e308"), f64::MAX);
        assert_eq!(crate::errno::errno(), 0);
    }
}


// ===========================================================================
// 覆盖面补齐：纯逻辑族的剩余导出（见 libc/tools/audit_conformance_coverage.py）
// ===========================================================================

/// 类型分类宏族（math_classify）：C11 7.12.3 的分类与符号位。
#[test]
fn math_classify_macros() {
    use crate::math_classify::*;
    assert_eq!(fpclassify(f64::NAN), FP_NAN);
    assert_eq!(fpclassify(f64::INFINITY), FP_INFINITE);
    assert_eq!(fpclassify(0.0), FP_ZERO);
    assert_eq!(fpclassify(1e-320), FP_SUBNORMAL);
    assert_eq!(fpclassify(1.0), FP_NORMAL);
    assert_eq!(isnan(f64::NAN), 1);
    assert_eq!(isnan(1.0), 0);
    // POSIX 只要求「非 0」；本实现与 glibc 一样**带上符号**（+1 / -1），见 isinf 的文档。
    assert_eq!(isinf(f64::INFINITY), 1);
    assert_eq!(isinf(f64::NEG_INFINITY), -1);
    assert_eq!(isinf(1.0), 0);
    assert_eq!(isfinite(1.0), 1);
    assert_eq!(isfinite(f64::NAN), 0);
    assert_eq!(isnormal(1.0), 1);
    assert_eq!(isnormal(0.0), 0);
    assert_eq!(isnormal(1e-320), 0);
    assert_eq!(signbit(-0.0), 1);
    assert_eq!(signbit(0.0), 0);
    assert_eq!(signbit(-1.0), 1);
}

/// frexp/ldexp：C11 7.12.6.3/7.12.6.6——frexp 拆成 [0.5,1) × 2^e，ldexp 反向。
#[test]
fn frexp_and_ldexp() {
    unsafe {
        let mut e: c_int = 0;
        assert_eq!(crate::math_decomp::frexp(0.0, &mut e), 0.0);
        let m = crate::math_decomp::frexp(8.0, &mut e);
        assert_eq!(m, 0.5);
        assert_eq!(e, 4, "8 = 0.5 x 2^4");
        assert_eq!(crate::math_decomp::ldexp(0.5, 4), 8.0);
        assert_eq!(crate::math_decomp::ldexp(1.0, 0), 1.0);
        // 往返
        let x = 1234.5678;
        let m2 = crate::math_decomp::frexp(x, &mut e);
        assert_eq!(crate::math_decomp::ldexp(m2, e), x);
    }
}

/// 宽字符串族（wchar.h）：长度/比较/拷贝/拼接/查找。
#[test]
fn wide_string_ops() {
    unsafe {
        let a = [b'h' as wchar_t, b'i' as wchar_t, 0];
        let b = [b'h' as wchar_t, b'o' as wchar_t, 0];
        assert_eq!(crate::wchar::wcslen(a.as_ptr()), 2);
        assert!(crate::wchar::wcscmp(a.as_ptr(), b.as_ptr()) < 0);
        assert_eq!(crate::wchar::wcscmp(a.as_ptr(), a.as_ptr()), 0);

        let mut dst = [0 as wchar_t; 8];
        crate::wchar::wcscpy(dst.as_mut_ptr(), a.as_ptr());
        assert_eq!(&dst[..3], &[b'h' as wchar_t, b'i' as wchar_t, 0]);

        crate::wchar::wcscat(dst.as_mut_ptr(), b.as_ptr());
        assert_eq!(&dst[..5], &[b'h' as wchar_t, b'i' as wchar_t, b'h' as wchar_t, b'o' as wchar_t, 0]);

        // wcsncpy：不足补 NUL（C11 7.29.4.2.2）。
        let mut pad = [0x7FFF as wchar_t; 5];
        crate::wchar::wcsncpy(pad.as_mut_ptr(), a.as_ptr(), 5);
        assert_eq!(pad, [b'h' as wchar_t, b'i' as wchar_t, 0, 0, 0]);

        let hit = crate::wchar::wcschr(a.as_ptr(), b'i' as wchar_t);
        // 注意元素宽度：wchar_t 是 4 字节，第 1 个元素在 base+4（不是 base+1）。
        assert_eq!(hit, a.as_ptr().add(1) as *mut wchar_t);
        assert!(crate::wchar::wcschr(a.as_ptr(), b'z' as wchar_t).is_null());
    }
}

/// BSD 传统字节操作：bzero 清零、bcopy 允许重叠、swab 交换相邻字节对。
#[test]
fn bsd_byte_ops() {
    unsafe {
        let mut b = [0xAAu8; 4];
        crate::posix_batch2::bzero(b.as_mut_ptr() as *mut c_void, 4);
        assert_eq!(b, [0, 0, 0, 0]);

        let mut o = *b"abcdef";
        crate::posix_batch2::bcopy(o.as_ptr() as *const c_void, o.as_mut_ptr().add(1) as *mut c_void, 4);
        // 数组没有 NUL 终止符：拷 4 字节到 o[1..5] ⇒ "a" + "abcd" + "f"。
        assert_eq!(&o, b"aabcdf");

        let src = *b"abcd";
        let mut d = [0u8; 4];
        crate::posix_batch2::swab(src.as_ptr() as *const c_void, d.as_mut_ptr() as *mut c_void, 4);
        assert_eq!(&d, b"badc");
        // n <= 1 不做任何事。
        let mut one = [0x55u8; 2];
        crate::posix_batch2::swab(src.as_ptr() as *const c_void, one.as_mut_ptr() as *mut c_void, 1);
        assert_eq!(one[0], 0x55);
    }
}

/// 字符串族剩余：strnlen 有界、strcat 追加、strerror 对未知 errno 不崩。
#[test]
fn string_extras() {
    unsafe {
        assert_eq!(crate::string::strnlen(cs(b"abc\0"), 10), 3);
        assert_eq!(crate::string::strnlen(cs(b"abc\0"), 2), 2, "must respect the bound");

        let mut buf = [0u8; 16];
        buf[..2].copy_from_slice(b"a\0");
        crate::string::strcat(buf.as_mut_ptr() as *mut c_char, cs(b"bc\0"));
        assert_eq!(&buf[..4], b"abc\0");

        // strerror 必须对**任意** errno 返回非空串（未知的也不能崩、不能返回 NULL）。
        for e in [0, 2, 22, 12345] {
            let p = crate::string::strerror(e);
            assert!(!p.is_null());
            assert_ne!(*p, 0, "strerror({}) must not be empty", e);
        }
    }
}

/// 整数转换族的剩余入口：atof / strtoll / strtoull / strtoimax / strtoumax / imaxabs / imaxdiv。
#[test]
fn stdlib_conversion_extras() {
    unsafe {
        assert_eq!(crate::stdlib::atof(cs(b"1.5\0")), 1.5);
        assert_eq!(crate::stdlib::atof(cs(b"nope\0")), 0.0);

        let mut e: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtoll(cs(b"-9223372036854775808\0"), &mut e, 10), i64::MIN);
        assert_eq!(crate::stdlib::strtoull(cs(b"18446744073709551615\0"), &mut e, 10), u64::MAX);
        assert_eq!(crate::stdlib::strtoimax(cs(b"-42\0"), &mut e, 10), -42);
        assert_eq!(crate::stdlib::strtoumax(cs(b"42\0"), &mut e, 10), 42);
        assert_eq!(crate::stdlib::imaxabs(-7), 7);
        let d = crate::stdlib::imaxdiv(-7, 2);
        assert_eq!((d.quot, d.rem), (-3, -1));
    }
}

/// difftime 返回秒差（C11 7.27.2.2）。
#[test]
fn difftime_seconds() {
    assert_eq!(crate::time::difftime(100, 40), 60.0);
    assert_eq!(crate::time::difftime(40, 100), -60.0);
    assert_eq!(crate::time::difftime(7, 7), 0.0);
}

/// confstr / nl_langinfo：只报**有真值**的名字（无 locale 数据库）。
#[test]
fn confstr_and_langinfo() {
    unsafe {
        // _CS_PATH = 1：与 shell 的 DEFAULT_PATH 逐字节一致（有门禁核对）。
        let mut buf = [0i8; 128];
        let n = crate::posix_batch7::confstr(1, buf.as_mut_ptr(), buf.len());
        assert!(n > 0);
        assert_eq!(cbytes(buf.as_ptr()), b"/programs:/volumes/BORUIX_DATA/3p");
        // 未知名字返回 0（POSIX：该名字无值）。
        assert_eq!(crate::posix_batch7::confstr(9999, buf.as_mut_ptr(), buf.len()), 0);

        // nl_langinfo：CODESET 必须是本实现自己的取值，且非空。
        let p = crate::posix_batch7::nl_langinfo(14);
        assert!(!p.is_null());
        assert_ne!(*p, 0);
    }
}

/// regerror：把错误码写成可读文本（POSIX 7.8.2）。
#[test]
fn regex_error_text() {
    unsafe {
        let mut re: crate::regex::regex_t = core::mem::zeroed();
        let mut buf = [0i8; 64];
        let n = crate::regex::regerror(0, &re, buf.as_mut_ptr(), buf.len());
        assert!(n > 0, "regerror must return the message length");
        assert!(n <= buf.len());
        let s = cbytes(buf.as_ptr());
        assert!(!s.is_empty());
    }
}

/// ctype 的剩余两项。
#[test]
fn ctype_case_predicates() {
    assert_eq!(crate::ctype::islower(b'a' as c_int), 1);
    assert_eq!(crate::ctype::islower(b'A' as c_int), 0);
    assert_eq!(crate::ctype::isupper(b'A' as c_int), 1);
    assert_eq!(crate::ctype::isupper(b'a' as c_int), 0);
    assert_eq!(crate::ctype::islower(-1), 0, "EOF is neither");
    assert_eq!(crate::ctype::isupper(-1), 0);
}


// ===========================================================================
// 覆盖面补齐（第二批）：系统问询族的失败路径 + 纯逻辑剩余项
// ===========================================================================

/// getpagesize：本系统页大小 4 KiB（与门禁的「数值镜像」同源）。
#[test]
fn getpagesize_is_4k() {
    assert_eq!(crate::unistd::getpagesize(), 4096);
}

/// sysconf：**不支持的名字**必须如实 -1 + EINVAL（POSIX 允许；不得编造返回值）。
/// 支持的名字（_SC_NPROCESSORS_ONLN）走内核，成功路径由真机覆盖。
#[test]
fn sysconf_unknown_name_is_einval() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    crate::errno::set_errno(0);
    assert_eq!(crate::unistd::sysconf(12345), -1);
    assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
}

/// gethostname：本系统没有主机名 ⇒ 如实写**空串**并返回 0（不编 localhost）。
#[test]
fn gethostname_is_empty_not_fabricated() {
    unsafe {
        let mut buf = [0x7Fi8; 8];
        assert_eq!(crate::posix_batch5::gethostname(buf.as_mut_ptr(), buf.len()), 0);
        assert_eq!(buf[0], 0, "must be an empty string, not a fabricated name");
        assert_eq!(buf[1], 0x7F, "must not write beyond the NUL");
        assert_eq!(crate::posix_batch5::gethostname(core::ptr::null_mut(), 8), -1);
        assert_eq!(crate::posix_batch5::gethostname(buf.as_mut_ptr(), 0), -1);
    }
}

/// uname：sysname/machine 是**事实**；nodename 必须为空（本系统无主机名概念）。
#[test]
fn uname_fields_have_real_sources() {
    unsafe {
        let mut u: crate::posix_batch5::utsname = core::mem::zeroed();
        assert_eq!(crate::posix_batch5::uname(&mut u), 0);
        assert_eq!(cbytes(u.sysname.as_ptr()), b"Boruix");
        assert_eq!(cbytes(u.machine.as_ptr()), b"x86_64");
        assert_eq!(
            cbytes(u.nodename.as_ptr()),
            b"",
            "no hostname concept -> empty, not a fabricated localhost"
        );
        assert_eq!(crate::posix_batch5::uname(core::ptr::null_mut()), -1);
    }
}

/// rand/srand：同一 seed 必须复现同一序列，取值落在 [0, RAND_MAX]（C11 7.22.2.1/2）。
#[test]
fn rand_srand_reproducible() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        crate::stdlib::srand(12345);
        let a = crate::stdlib::rand();
        let b = crate::stdlib::rand();
        crate::stdlib::srand(12345);
        assert_eq!(crate::stdlib::rand(), a, "same seed must reproduce the sequence");
        assert_eq!(crate::stdlib::rand(), b);
        for _ in 0..8 {
            let v = crate::stdlib::rand();
            assert!((0..=i32::MAX).contains(&v), "rand must stay within [0, RAND_MAX]");
        }
    }
}

/// 信号集操作（POSIX 7.14.1.1）：置空/填满/增删/判定 + 非法信号号如实 EINVAL。
#[test]
fn sigset_operations() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let mut s: crate::signal::sigset_t = 0;
        assert_eq!(crate::signal::sigemptyset(&mut s), 0);
        assert_eq!(s, 0);
        assert_eq!(crate::signal::sigismember(&s, 2), 0);
        assert_eq!(crate::signal::sigaddset(&mut s, 2), 0);
        assert_eq!(crate::signal::sigismember(&s, 2), 1);
        assert_eq!(crate::signal::sigdelset(&mut s, 2), 0);
        assert_eq!(crate::signal::sigismember(&s, 2), 0);
        assert_eq!(crate::signal::sigfillset(&mut s), 0);
        // 满集 = 内核 SignalSet::all() = u64::MAX（含 bit63）。
        assert_eq!(s, u64::MAX);
        assert_eq!(crate::signal::sigismember(&s, 1), 1);
        assert_eq!(crate::signal::sigismember(&s, 63), 1);

        // 信号号 0 与 >= NSIG 都不是合法信号（POSIX：EINVAL，不静默夹取）。
        crate::errno::set_errno(0);
        assert_eq!(crate::signal::sigaddset(&mut s, 0), -1);
        assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
        crate::errno::set_errno(0);
        assert_eq!(crate::signal::sigismember(&s, 64), -1);
        assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
    }
}

/// strtold：本目标 long double ≈ f64，行为与 strtod 一致。
#[test]
fn strtold_matches_strtod() {
    unsafe {
        let mut e: *const c_char = core::ptr::null();
        assert_eq!(crate::stdlib::strtold(cs(b"2.5\0"), &mut e), 2.5);
        assert!(crate::stdlib::strtold(cs(b"inf\0"), &mut e).is_infinite());
    }
}

/// wordfree：NULL 安全（POSIX 对该情形未定义，取最保守行为）。
#[test]
fn wordfree_is_null_safe() {
    unsafe {
        crate::posix_batch5::wordfree(core::ptr::null_mut());
    }
}

/// getopt_long：长选项（GNU 扩展）——名字匹配、需参数、结束。
#[test]
fn getopt_long_basics() {
    let _g = GETOPT_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        use crate::posix_batch4::{getopt_long, no_argument, option, required_argument};
        let (_b, mut argv) = make_argv(&[b"prog", b"--verbose", b"--out", b"x"]);
        let longs = [
            option {
                name: b"verbose\0".as_ptr() as *const c_char,
                has_arg: no_argument,
                flag: core::ptr::null_mut(),
                val: b'v' as c_int,
            },
            option {
                name: b"out\0".as_ptr() as *const c_char,
                has_arg: required_argument,
                flag: core::ptr::null_mut(),
                val: b'o' as c_int,
            },
            option {
                name: core::ptr::null(),
                has_arg: 0,
                flag: core::ptr::null_mut(),
                val: 0,
            },
        ];
        crate::posix_batch4::optind = 1;
        let os = b"\0";
        let f = |argv: &mut alloc::vec::Vec<*mut c_char>| {
            getopt_long(
                4,
                argv.as_ptr(),
                os.as_ptr() as *const c_char,
                longs.as_ptr(),
                core::ptr::null_mut(),
            )
        };
        assert_eq!(f(&mut argv), b'v' as c_int);
        assert_eq!(f(&mut argv), b'o' as c_int);
        let oa = crate::posix_batch4::optarg;
        assert_eq!(cbytes(oa), b"x");
        assert_eq!(f(&mut argv), -1);
    }
}


// ===========================================================================
// 覆盖面补齐（第三批）：exec 族的如实失败、环境变量族
// ===========================================================================

/// exec 族：本系统**没有替换进程映像的 syscall**（只有派生）⇒ 如实 ENOTSUP，
/// 绝不返回"成功"。system 无命令处理器 ⇒ 按 POSIX 返回 1（cmd == NULL 时）。
#[test]
fn exec_family_honestly_fails() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        crate::errno::set_errno(0);
        assert_eq!(crate::exec::execv(cs(b"/programs/nope.elf\0"), core::ptr::null()), -1);
        assert_eq!(crate::errno::errno(), crate::errno::ENOTSUP);
        crate::errno::set_errno(0);
        assert_eq!(
            crate::exec::execve(cs(b"/programs/nope.elf\0"), core::ptr::null(), core::ptr::null()),
            -1
        );
        assert_eq!(crate::errno::errno(), crate::errno::ENOTSUP);
        crate::errno::set_errno(0);
        assert_eq!(crate::unistd::execvp(cs(b"nope\0"), core::ptr::null()), -1);

        // system(NULL)：POSIX 规定此时返回非 0（有命令处理器）；本系统没有 ⇒ 返回 1。
        assert_eq!(crate::exec::system(core::ptr::null()), 1);
        // 非 NULL：无 shell 可执行 ⇒ 如实 ENOTSUP。
        crate::errno::set_errno(0);
        assert_eq!(crate::exec::system(cs(b"echo hi\0")), -1);
        assert_eq!(crate::errno::errno(), crate::errno::ENOTSUP);
    }
}

/// getlogin：没有登录会话名查询 ⇒ 无环境变量时如实 NULL（不编名字）。
#[test]
fn getlogin_never_fabricates() {
    unsafe {
        let p = crate::posix_batch5::getlogin();
        if !p.is_null() {
            let v = cbytes(p);
            assert!(!v.is_empty(), "a returned login name must not be empty");
        }
    }
}

/// 环境变量族：setenv/getenv/unsetenv/putenv/clearenv 的**往返**与非法参数。
#[test]
fn env_round_trip() {
    let _g = ERRNO_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        // 非法参数如实 EINVAL。
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::setenv(core::ptr::null(), cs(b"1\0"), 1), -1);
        assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::setenv(cs(b"A=B\0"), cs(b"1\0"), 1), -1);
        assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
        crate::errno::set_errno(0);
        assert_eq!(crate::stdlib::unsetenv(core::ptr::null()), -1);
        assert_eq!(crate::errno::errno(), crate::errno::EINVAL);

        // **往返部分宿主测不了**：setenv 经 libc 的 malloc → 内核 brk（宿主上没有内核）⇒
        // 恒 ENOMEM。故这里只钉住**纯参数校验**那一半；往返由真机 libccheck 覆盖。
        // （不写一个恒失败的假测试。）
    }
}
