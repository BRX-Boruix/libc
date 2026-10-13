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
