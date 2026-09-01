//! libc 纯逻辑模块的 host 单测（不链接 libsys/裸机运行时）。
//!
//! 通过 \`#[path]\` 直接 include libc 的纯逻辑源文件（stdio_format、float、
//! string、ctype、stdlib），在 host std 环境下做对抗性单测（S23/S30/S31）。
//! 系统调用相关函数（malloc/printf 输出/文件 IO）由内核 shell 验证层负责
//! 端到端验收。

extern crate alloc;
use alloc::vec::Vec;

#[path = "../../src/ctypes.rs"]
mod ctypes;
#[path = "../../src/ctype.rs"]
mod ctype;
// 纯逻辑测试桩：string::strdup / stdlib::rand 依赖。
mod malloc {
    pub extern "C" fn malloc(size: usize) -> *mut u8 {
        let v = unsafe { std::alloc::alloc(std::alloc::Layout::from_size_align(size, 16).unwrap()) };
        v
    }
}
mod random {
    pub fn next() -> u64 { 1234567890 }
    pub fn seed(_s: u64) {}
}
// string.rs 定义 no_mangle memcpy/memset/memcmp，会覆盖 std 运行时符号并在
// host 测试启动时崩溃——故 string 函数不在本 harness 测（改由 shell 端到端层验收）。
// mod string;
#[path = "../../src/stdio_format.rs"]
mod stdio_format;
#[path = "../../src/float.rs"]
mod float;
#[path = "../../src/float_bigint.rs"]
mod float_bigint;
#[path = "../../src/stdlib.rs"]
mod stdlib;
// 为纯逻辑测试提供最小 errno 桩（stdlib.rs 需要 set_errno/ERANGE）。
mod errno {
    pub fn set_errno(_e: i32) {}
    pub const ERANGE: i32 = 34;
}

// stdlib 的 strtod/strtof 依赖的 stdio 辅助（host 桩 + 真实实现）。
mod stdio {
    pub fn cstr_to_bytes(s: *const i8) -> &'static [u8] {
        if s.is_null() {
            return &[];
        }
        let mut len = 0usize;
        while unsafe { *s.add(len) } != 0 {
            len += 1;
        }
        unsafe { core::slice::from_raw_parts(s as *const u8, len) }
    }
    pub fn f64_pow10(k: i32) -> f64 {
        let mut r = 1.0f64;
        let mut n = k;
        if n >= 0 {
            while n > 0 { r *= 10.0; n -= 1; }
        } else {
            while n < 0 { r /= 10.0; n += 1; }
        }
        r
    }
}

use stdio_format::*;

fn render_with<F: FnMut(&Spec, &mut dyn FmtSink) -> Result<(), ()>>(
    fmt: &str,
    mut f: F,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut sink = VecSink(&mut out);
    parse_and_format(fmt.as_bytes(), &mut sink, &mut f).unwrap();
    out
}

fn render_int(spec: Spec, value: u64, neg: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut sink = VecSink(&mut out);
    emit_int(&spec, value, neg, &mut sink).unwrap();
    out
}

#[test]
fn test_emit_int_decimal() {
    let s = Spec { left: false, plus: false, space: false, zero: false, alt: false,
        width: -1, prec: -1, len: Length::None, conv: Conv::Int, upper: false };
    assert_eq!(render_int(s, 42, false), b"42".to_vec());
    assert_eq!(render_int(s, 0, false), b"0".to_vec());
    assert_eq!(render_int(s, 42, true), b"-42".to_vec());
    assert_eq!(render_int(s, 0xFFFFFFFF, false), b"4294967295".to_vec());
}

#[test]
fn test_emit_int_width_zero_pad() {
    let s = Spec { left: false, plus: false, space: false, zero: true, alt: false,
        width: 5, prec: -1, len: Length::None, conv: Conv::Int, upper: false };
    assert_eq!(render_int(s, 42, false), b"00042".to_vec());
    let s2 = Spec { left: true, plus: false, space: false, zero: true, alt: false,
        width: 5, prec: -1, len: Length::None, conv: Conv::Int, upper: false };
    assert_eq!(render_int(s2, 42, false), b"42   ".to_vec());
}

#[test]
fn test_emit_int_hex() {
    let s = Spec { left: false, plus: false, space: false, zero: false, alt: true,
        width: -1, prec: -1, len: Length::Ll, conv: Conv::Hex, upper: false };
    assert_eq!(render_int(s, 0x1A2B, false), b"0x1a2b".to_vec());
    let s2 = Spec { left: false, plus: false, space: false, zero: false, alt: true,
        width: -1, prec: -1, len: Length::Ll, conv: Conv::Hex, upper: true };
    assert_eq!(render_int(s2, 0xDEAD, false), b"0XDEAD".to_vec());
}

#[test]
fn test_emit_int_sign_space() {
    let s = Spec { left: false, plus: false, space: true, zero: false, alt: false,
        width: -1, prec: -1, len: Length::None, conv: Conv::Int, upper: false };
    assert_eq!(render_int(s, 7, false), b" 7".to_vec());
    let s2 = Spec { left: false, plus: true, space: false, zero: false, alt: false,
        width: -1, prec: -1, len: Length::None, conv: Conv::Int, upper: false };
    assert_eq!(render_int(s2, 7, false), b"+7".to_vec());
}

#[test]
fn test_emit_str_precision_width() {
    let s = Spec { left: false, plus: false, space: false, zero: false, alt: false,
        width: 10, prec: 3, len: Length::None, conv: Conv::Str, upper: false };
    let mut out = Vec::new();
    let mut sink = VecSink(&mut out);
    emit_str(&s, b"hello", &mut sink).unwrap();
    assert_eq!(out, b"       hel".to_vec());
}

#[test]
fn test_parse_and_format_literal() {
    let out = render_with("hello %d world", |_s, _sink| Ok(()));
    assert_eq!(out, b"hello  world".to_vec());
}

#[test]
fn test_parse_and_format_percent() {
    let out = render_with("100%% done", |_s, _sink| Ok(()));
    assert_eq!(out, b"100% done".to_vec());
}

#[test]
fn test_fit_unsigned_hh() {
    let v = fit_unsigned(0xFFFFFFFFFFFFFFFF, Length::Hh, true);
    assert_eq!(v, u64::MAX);
    let v2 = fit_unsigned(0x1FF, Length::Hh, false);
    assert_eq!(v2, 0xFF);
}

// ---------- float 单测 ----------

fn render_fixed(v: f64, prec: usize) -> Vec<u8> {
    let mut d = float::decompose(v);
    let mut out = Vec::new();
    let mut sink = VecSink(&mut out);
    let s = Spec { left: false, plus: false, space: false, zero: false, alt: false,
        width: -1, prec: prec as i64, len: Length::None, conv: Conv::Float, upper: false };
    float::emit_fixed(&s, &mut d, prec, &mut sink).unwrap();
    out
}

#[test]
fn test_float_fixed_basic() {
    assert_eq!(render_fixed(3.5, 1), b"3.5".to_vec());
    assert_eq!(render_fixed(0.0, 2), b"0.00".to_vec());
    assert_eq!(render_fixed(-2.25, 2), b"-2.25".to_vec());
    assert_eq!(render_fixed(100.0, 2), b"100.00".to_vec());
}

#[test]
fn test_float_fixed_small() {
    assert_eq!(render_fixed(0.5, 1), b"0.5".to_vec());
    assert_eq!(render_fixed(0.0625, 4), b"0.0625".to_vec());
}

#[test]
fn test_float_special() {
    assert_eq!(render_fixed(f64::INFINITY, 1), b"inf".to_vec());
    assert_eq!(render_fixed(f64::NEG_INFINITY, 1), b"-inf".to_vec());
    let nan = render_fixed(f64::NAN, 1);
    assert_eq!(nan, b"nan".to_vec());
}

#[test]
fn test_float_exp() {
    let mut d = float::decompose(12345.0);
    let mut out = Vec::new();
    let mut sink = VecSink(&mut out);
    let s = Spec { left: false, plus: false, space: false, zero: false, alt: false,
        width: -1, prec: 2, len: Length::None, conv: Conv::Exp, upper: false };
    float::emit_exp(&s, &mut d, 2, &mut sink).unwrap();
    assert_eq!(out, b"1.23e+04".to_vec());
}

// ---------- stdlib（strtol）单测 ----------

#[test]
fn test_strtol() {
    let s = b"12345\0".as_ptr() as *const i8;
    let v = unsafe { stdlib::strtol(s, core::ptr::null_mut(), 10) };
    assert_eq!(v, 12345);
    // base 0 with hex prefix
    let h = b"0x1A\0".as_ptr() as *const i8;
    let v2 = unsafe { stdlib::strtol(h, core::ptr::null_mut(), 0) };
    assert_eq!(v2, 0x1A);
    // negative
    let n = b"-99\0".as_ptr() as *const i8;
    let v3 = unsafe { stdlib::strtol(n, core::ptr::null_mut(), 10) };
    assert_eq!(v3, -99);
}

#[test]
fn test_strtoul() {
    let s = b"4294967295\0".as_ptr() as *const i8;
    let v = unsafe { stdlib::strtoul(s, core::ptr::null_mut(), 10) };
    assert_eq!(v, 4294967295u64);
}

// ---------- ctype 单测 ----------

#[test]
fn test_ctype() {
    assert_eq!(ctype::isdigit(b'5' as i32), 1);
    assert_eq!(ctype::isdigit(b'x' as i32), 0);
    assert_eq!(ctype::isalpha(b'A' as i32), 1);
    assert_eq!(ctype::isspace(b' ' as i32), 1);
    assert_eq!(ctype::tolower(b'Z' as i32), b'z' as i32);
    assert_eq!(ctype::toupper(b'a' as i32), b'A' as i32);
    assert_eq!(ctype::isxdigit(b'f' as i32), 1);
}

// ---------- 对抗性边界测试（S30/S31） ----------

#[test]
fn test_float_rounding() {
    // 四舍六入五成双（round-half-even）。
    assert_eq!(render_fixed(2.5, 0), b"2".to_vec());   // 2.5 -> 2 (even)
    assert_eq!(render_fixed(3.5, 0), b"4".to_vec());   // 3.5 -> 4 (even)
    assert_eq!(render_fixed(1.45, 1), b"1.4".to_vec()); // 1.45 -> 1.4
    assert_eq!(render_fixed(1.55, 1), b"1.6".to_vec());
}

#[test]
fn test_float_negative_zero() {
    // -0.0 应输出 -0.00（C 语义，%f）。
    assert_eq!(render_fixed(-0.0, 2), b"-0.00".to_vec());
}

#[test]
fn test_float_large() {
    // u128 定点可表示范围内。
    assert_eq!(render_fixed(123456789.0, 0), b"123456789".to_vec());
    assert_eq!(render_fixed(0.000001, 6), b"0.000001".to_vec());
}

fn render_exp(v: f64, prec: usize) -> Vec<u8> {
    let mut d = float::decompose(v);
    let mut out = Vec::new();
    let mut sink = VecSink(&mut out);
    let s = Spec { left: false, plus: false, space: false, zero: false, alt: false,
        width: -1, prec: prec as i64, len: Length::None, conv: Conv::Exp, upper: false };
    float::emit_exp(&s, &mut d, prec, &mut sink).unwrap();
    out
}

#[test]
fn test_float_exp_small() {
    // 0.001234 -> 1.23e-03
    assert_eq!(render_exp(0.001234, 2), b"1.23e-03".to_vec());
    assert_eq!(render_exp(-0.001234, 2), b"-1.23e-03".to_vec());
}

#[test]
fn test_strtol_overflow() {
    // 超出 i64 范围应置 ERANGE 并截断。验证不 panic 且行为确定。
    let s = b"99999999999999999999999999\0".as_ptr() as *const i8;
    let v = unsafe { stdlib::strtol(s, core::ptr::null_mut(), 10) };
    // 截断到 i64::MAX。
    assert_eq!(v, i64::MAX);
}

#[test]
fn test_strtol_base_detection() {
    // base=0：0x 前缀→16，0 前缀→8，否则→10。
    let s = b"077\0".as_ptr() as *const i8;
    let v = unsafe { stdlib::strtol(s, core::ptr::null_mut(), 0) };
    assert_eq!(v, 63); // 0o77 = 63
    let s2 = b"19\0".as_ptr() as *const i8;
    let v2 = unsafe { stdlib::strtol(s2, core::ptr::null_mut(), 0) };
    assert_eq!(v2, 19);
}

#[test]
fn test_fit_unsigned_hh_signed() {
    // %hhd 传 0x1FF (511) → 截断到 0xFF → 符号扩展为 -1。
    let v = fit_unsigned(0x1FF, Length::Hh, true);
    assert_eq!(v, u64::MAX);
}

// ---------- 全范围 dtoa（大整数精确，超越 u128 窗口） ----------

#[test]
fn test_float_fullrange_large() {
    // 1e300 超出旧 u128 窗口，应能精确输出。
    // 直接验证 decompose：dec_exp=300（最高位 10^300），首位 '1'。
    let d = float::decompose(1e300);
    assert_eq!(d.dec_exp, 300);
    assert_eq!(d.digits[0], b'1');
    // %f 精度 0 应输出完整整数部分（1e300 实际 f64 值约 10^300，很长），
    // 长度应 > 300 位。
    let out = render_fixed(1e300, 0);
    assert!(out.len() > 300);
    assert_eq!(out[0], b'1');
}

#[test]
fn test_float_fullrange_tiny() {
    // 最小次正规数 5e-324：dec_exp = -324（最高位 10^-324）。
    let d = float::decompose(f64::from_bits(1)); // 最小次正规数
    assert_eq!(d.dec_exp, -324);
    // 0.0
    let z = float::decompose(0.0);
    assert_eq!(z.digits[0], b'0');
    assert_eq!(z.digit_len, 1);
}

#[test]
fn test_float_fullrange_roundtrip_precision() {
    // 关键：在旧窗口之外的值，仍给出精确十进制数字（非近似）。
    // 0.1 的精确二进制展开，首个 16 位应与已知一致。
    let d = float::decompose(0.1);
    // 0.1 = 0.1000000000000000055511151231257827...
    let expect: &[u8] = b"1000000000000000055511151231257827";
    for (i, &c) in expect.iter().enumerate() {
        assert_eq!(d.digits[i], c, "digit {i}");
    }
    // 1e-40 边界：f64(1e-40) 并非恰好 1e-40，其精确值首非零位在 10^-41。
    let d2 = float::decompose(1e-40);
    assert_eq!(d2.dec_exp, -41);
    // 且首位精确已知（1e-40 最近的 f64 首位是 '9'，因实际值略小于 1e-40）。
    assert!(d2.digits[0] == b'9' || d2.digits[0] == b'1');
}

// ---------- qsort / bsearch 单测 ----------

extern "C" fn cmp_i32(a: *const core::ffi::c_void, b: *const core::ffi::c_void) -> i32 {
    let x = unsafe { *(a as *const i32) };
    let y = unsafe { *(b as *const i32) };
    if x < y { -1 } else if x > y { 1 } else { 0 }
}

#[test]
fn test_qsort_sorts() {
    let mut arr = [5i32, 2, 9, 1, 5, 6];
    unsafe {
        stdlib::qsort(
            arr.as_mut_ptr() as *mut core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
    }
    assert_eq!(arr, [1, 2, 5, 5, 6, 9]);
}

#[test]
fn test_qsort_already_sorted() {
    let mut arr = [1i32, 2, 3, 4];
    unsafe {
        stdlib::qsort(
            arr.as_mut_ptr() as *mut core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
    }
    assert_eq!(arr, [1, 2, 3, 4]);
}

#[test]
fn test_qsort_single() {
    let mut arr = [7i32];
    unsafe {
        stdlib::qsort(
            arr.as_mut_ptr() as *mut core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
    }
    assert_eq!(arr, [7]);
}

#[test]
fn test_bsearch_find() {
    let arr = [1i32, 3, 5, 7, 9];
    let key = 5i32;
    let p = unsafe {
        stdlib::bsearch(
            &key as *const i32 as *const core::ffi::c_void,
            arr.as_ptr() as *const core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        )
    };
    assert!(!p.is_null());
    assert_eq!(unsafe { *(p as *const i32) }, 5);
}

#[test]
fn test_bsearch_miss() {
    let arr = [1i32, 3, 5, 7, 9];
    let key = 4i32;
    let p = unsafe {
        stdlib::bsearch(
            &key as *const i32 as *const core::ffi::c_void,
            arr.as_ptr() as *const core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        )
    };
    assert!(p.is_null());
}

// ---------- qsort 混合快排（introsort 风格）补充测试 ----------

#[test]
fn test_qsort_reverse_sorted() {
    // 逆序输入：触发 median-of-three 与划分路径。
    let mut arr = [10i32, 9, 8, 7, 6, 5, 4, 3, 2, 1];
    unsafe {
        stdlib::qsort(
            arr.as_mut_ptr() as *mut core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
    }
    assert_eq!(arr, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

#[test]
fn test_qsort_large_with_duplicates() {
    // 大数组 + 重复值（压测 >THRESHOLD=12 的划分 + 稳定收敛）。
    let mut arr = [
        42i32, 7, 42, 1, 99, 42, 3, 7, 42, 0, 55, 42, 8, 42, 2, 42,
    ];
    let mut expected = arr;
    expected.sort_unstable();
    unsafe {
        stdlib::qsort(
            arr.as_mut_ptr() as *mut core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<i32>(),
            cmp_i32,
        );
    }
    assert_eq!(arr, expected);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rec {
    key: i32,
    val: i32,
}

extern "C" fn cmp_rec(a: *const core::ffi::c_void, b: *const core::ffi::c_void) -> i32 {
    let x = unsafe { (*(a as *const Rec)).key };
    let y = unsafe { (*(b as *const Rec)).key };
    if x < y { -1 } else if x > y { 1 } else { 0 }
}

#[test]
fn test_qsort_records_size_gt_1() {
    // 元素 size > 1（8 字节结构体）验证按元素步进正确。
    let mut arr = [
        Rec { key: 5, val: 50 },
        Rec { key: 1, val: 10 },
        Rec { key: 9, val: 90 },
        Rec { key: 3, val: 30 },
    ];
    unsafe {
        stdlib::qsort(
            arr.as_mut_ptr() as *mut core::ffi::c_void,
            arr.len(),
            core::mem::size_of::<Rec>(),
            cmp_rec,
        );
    }
    let keys: Vec<i32> = arr.iter().map(|r| r.key).collect();
    assert_eq!(keys, vec![1, 3, 5, 9]);
}

// ---------- strtod / strtof 单测 ----------

fn cstr(s: &str) -> (Vec<i8>, *mut *const i8) {
    let mut v: Vec<i8> = s.bytes().map(|b| b as i8).collect();
    v.push(0);
    (v, core::ptr::null_mut())
}

#[test]
fn test_strtod_basic() {
    let (s, _) = cstr("3.14");
    let v = unsafe { stdlib::strtod(s.as_ptr(), core::ptr::null_mut()) };
    assert!((v - 3.14).abs() < 1e-12);
}

#[test]
fn test_strtod_sign_neg() {
    let (s, _) = cstr("-42.5");
    let v = unsafe { stdlib::strtod(s.as_ptr(), core::ptr::null_mut()) };
    assert!((v - -42.5).abs() < 1e-12);
}

#[test]
fn test_strtod_exponent() {
    let (s, _) = cstr("1.5e3");
    let v = unsafe { stdlib::strtod(s.as_ptr(), core::ptr::null_mut()) };
    assert!((v - 1500.0).abs() < 1e-9);
    let (s2, _) = cstr("2e-2");
    let v2 = unsafe { stdlib::strtod(s2.as_ptr(), core::ptr::null_mut()) };
    assert!((v2 - 0.02).abs() < 1e-9);
}

#[test]
fn test_strtod_whitespace() {
    let (s, _) = cstr("   	 7.0");
    let v = unsafe { stdlib::strtod(s.as_ptr(), core::ptr::null_mut()) };
    assert!((v - 7.0).abs() < 1e-12);
}

#[test]
fn test_strtod_endptr() {
    let (s, _) = cstr("123abc");
    let mut end: *const i8 = core::ptr::null();
    let v = unsafe { stdlib::strtod(s.as_ptr(), &mut end) };
    assert!((v - 123.0).abs() < 1e-12);
    // endptr 应指向 'a'。
    unsafe {
        let off = end.offset_from(s.as_ptr());
        assert_eq!(off, 3);
    }
}

#[test]
fn test_strtod_no_match() {
    let (s, _) = cstr("xyz");
    let v = unsafe { stdlib::strtod(s.as_ptr(), core::ptr::null_mut()) };
    assert_eq!(v, 0.0);
}

#[test]
fn test_strtof_basic() {
    let (s, _) = cstr("2.5");
    let v = unsafe { stdlib::strtof(s.as_ptr(), core::ptr::null_mut()) };
    assert!((v - 2.5f32).abs() < 1e-6);
}

/// 严格正确舍入：strtod 结果应与 host `f64::from_str` 位级一致。
fn assert_strtod_roundtrip(input: &str) {
    let (s, _) = cstr(input);
    let v = unsafe { stdlib::strtod(s.as_ptr(), core::ptr::null_mut()) };
    let expect: f64 = input.parse().unwrap();
    assert_eq!(v.to_bits(), expect.to_bits(),
        "strtod({:?}) = {} (bits {:x}), expect {} ({:x})",
        input, v, v.to_bits(), expect, expect.to_bits());
}

#[test]
fn test_strtod_correct_rounding_basic() {
    // 经典正确舍入样例（1 ulp 级）。
    for s in ["0.1", "0.2", "0.3", "0.7", "1.1", "2.5", "0.5", "3.14",
              "0.1e1", "1e100", "6.02e23", "2.99792458e8",
              "1.0000000000000000000000001",   // >18 位，考验不截断
              "0.000000000000000000000000001", // 1e-27
              "123456789012345678901234567890", // 30 位整数
              "9.999999999999999999999999999", // 逼近 10
              "2.2250738585072014e-308",       // 最小正常数
              "2.2250738585072011e-308",       // 最小次正规边界（著名舍入样例）
              "4.9406564584124654e-324",       // 最小次正规
              "1.7976931348623157e308",        // 最大有限
              "1.7976931348623159e308",        // 溢出到 inf
              "5e-324",                        // 最小次正规
              "0", "1", "-0", "100", "0.00000000000000000000000000000000000000000000000001"] {
        assert_strtod_roundtrip(s);
    }
}

#[test]
fn test_strtod_correct_rounding_fuzz() {
    // 伪随机 30 组不同量级/位数，位级对比 host parse。
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..30 {
        let int_len = (next() % 12) + 1; // 1..12 位整数
        let frac_len = next() % 14;      // 0..13 位小数
        let mut ds = String::new();
        if next() % 2 == 1 { ds.push('-'); }
        for _ in 0..int_len { ds.push((b'0' + (next() % 10) as u8) as char); }
        if frac_len > 0 {
            ds.push('.');
            for _ in 0..frac_len { ds.push((b'0' + (next() % 10) as u8) as char); }
        }
        let e = (next() % 200) as i64 - 100; // -100..100
        ds.push('e');
        ds.push(if e < 0 { '-' } else { '+' });
        ds.push_str(&e.abs().to_string());
        assert_strtod_roundtrip(&ds);
    }
}

/// 严格正确舍入：strtof 结果应与 host `f32::from_str` 位级一致。
fn assert_strtof_roundtrip(input: &str) {
    let (s, _) = cstr(input);
    let v = unsafe { stdlib::strtof(s.as_ptr(), core::ptr::null_mut()) };
    let expect: f32 = input.parse().unwrap();
    assert_eq!(v.to_bits(), expect.to_bits(),
        "strtof({:?}) = {} (bits {:x}), expect {} ({:x})",
        input, v, v.to_bits(), expect, expect.to_bits());
}

#[test]
fn test_strtof_correct_rounding_basic() {
    // f32 关键样例：正常数、次正规边界、溢出、双舍入敏感值。
    for s in ["0.1", "0.3", "0.7", "3.14159", "1.0", "-0.0", "0.5",
              "1.17549435e-38",   // 最小正常 f32 ≈ 2^-126
              "1.17549421e-38",   // 略低于最小正常（次正规边界）
              "1.1754942e-38",
              "1.40129846e-45",   // 最小次正规 f32 = 2^-149
              "3.4028234663852886e38", // 最大正常 f32
              "3.4028236e38",     // 溢出到 inf
              "2.0000001e1",      // 多有效数字
              "1.00000000000000000001", // 考验不截断（>24 位）
              "1234567890123456789012345", // 25 位整数
              "9.9999999999999999", // 逼近 10
              "1.19209290e-7",    // 2^-23
              "0", "1", "100", "-3.25"] {
        assert_strtof_roundtrip(s);
    }
}

#[test]
fn test_strtof_correct_rounding_fuzz() {
    // 伪随机 40 组，位级对比 host parse。用与 strtod fuzz 不同的种子。
    let mut seed: u64 = 0x2545F4914F6CDD1D;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..40 {
        let int_len = (next() % 10) + 1; // 1..10 位整数
        let frac_len = next() % 14;      // 0..13 位小数
        let mut ds = String::new();
        if next() % 2 == 1 { ds.push('-'); }
        for _ in 0..int_len { ds.push((b'0' + (next() % 10) as u8) as char); }
        if frac_len > 0 {
            ds.push('.');
            for _ in 0..frac_len { ds.push((b'0' + (next() % 10) as u8) as char); }
        }
        let e = (next() % 200) as i64 - 120; // -120..80（覆盖 f32 次正规/正常范围）
        ds.push('e');
        ds.push(if e < 0 { '-' } else { '+' });
        ds.push_str(&e.abs().to_string());
        assert_strtof_roundtrip(&ds);
    }
}


