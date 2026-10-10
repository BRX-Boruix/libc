//! 时间函数（C ABI）：time / clock / sleep / clock_gettime 简化。
//!
//! 真实数据链路（S06）：
//! - \`time()\` 返回 Unix epoch 秒（wall clock），来自 SysFS \`/system/info/time\`
//!   （内核 RTC 直读 CMOS）。SysFS 不可用时返回 (time_t)-1 置 errno（S09）。
//! - \`clock()\` 返回单调纳秒（内核 uptime_ms），换算为 CLOCKS_PER_SEC 单位。
//! - \`sleep\` 经 libsys \`sleep(ns)\` 阻塞。

use crate::ctypes::{c_char, c_int, c_ulong, size_t};
use crate::errno::{set_errno, from_libsys};

/// \`time_t\`：秒级时间类型（64 位）。
pub type time_t = i64;
/// \`clock_t\`：时钟计数类型。
pub type clock_t = i64;

/// 每秒时钟滴答数（与 \`clock()\` 返回值换算；本实现用纳秒）。
pub const CLOCKS_PER_SEC: clock_t = 1_000_000_000;

/// difftime(t1, t0)：两个时刻之差（秒，double）。
///
/// 来路（3P6-2 第二波「整项缺失」类，反向对账列出）。**为什么返回 double**：POSIX 规定如此
/// ——整数减法在 time_t 为 32 位时可能溢出，用浮点差是标准要求的语义。
#[unsafe(no_mangle)]
pub extern "C" fn difftime(t1: time_t, t0: time_t) -> f64 {
    (t1 as f64) - (t0 as f64)
}

/// CLOCK_REALTIME / CLOCK_MONOTONIC（取值与 Linux 一致）。
pub const CLOCK_REALTIME: c_int = 0;
pub const CLOCK_MONOTONIC: c_int = 1;

/// clock_gettime(clk, tp)：取时钟。
///
/// **来路（3P6-2 第二波「整项缺失」类，反向对账列出）。**
///
/// - `CLOCK_REALTIME`：真实挂钟（内核 RTC 直读 CMOS）。
/// - `CLOCK_MONOTONIC`：内核单调 uptime（`libsys::now()`——与 shell 的 `now`、
///   `clock()` **同一真值来源**，不走挂钟）。
///
/// **粒度如实声明（S09）**：本系统经 SysFS 暴露的单调时钟是**毫秒**级
/// （`INFO_BOOT_MS`），故 `tv_nsec` 恒为 1_000_000 的整数倍。
/// 这**比返回 EINVAL 好**：POSIX 程序（超时、耗时统计）拿到 EINVAL 会直接失败，
/// 而毫秒级单调钟是**可用**的；同时**不**用挂钟冒充单调钟——那会让「测量耗时」
/// 的代码在系统时间被调整时给出错的结果。
/// - 其余时钟：如实 `EINVAL`（POSIX 允许对不支持的时钟报 EINVAL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_gettime(clk: c_int, tp: *mut Timespec) -> c_int {
    unsafe {
        if tp.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        if clk == CLOCK_MONOTONIC {
            let ns = libsys::now();
            (*tp).tv_sec = (ns / 1_000_000_000) as i64;
            (*tp).tv_nsec = (ns % 1_000_000_000) as i64;
            return 0;
        }
        if clk != CLOCK_REALTIME {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        let mut tv = Timeval { tv_sec: 0, tv_usec: 0 };
        if gettimeofday(&mut tv, core::ptr::null_mut()) != 0 {
            return -1;
        }
        (*tp).tv_sec = tv.tv_sec;
        (*tp).tv_nsec = tv.tv_usec * 1000;
        0
    }
}

/// asctime(tm)：固定格式 "Www Mmm dd hh:mm:ss yyyy\n"（26 字节，含结尾换行）。
///
/// **诚实边界**：返回**静态缓冲**（POSIX 允许；ctime 与它共用），故不可重入、下次调用会覆盖。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn asctime(t: *const Tm) -> *mut c_char {
    static mut ASCTIME_BUF: [c_char; 32] = [0; 32];
    unsafe {
        let buf = core::ptr::addr_of_mut!(ASCTIME_BUF) as *mut c_char;
        if t.is_null() {
            *buf = 0;
            return buf;
        }
        strftime(
            buf,
            26,
            b"%a %b %e %H:%M:%S %Y\n\0".as_ptr() as *const c_char,
            t,
        );
        buf
    }
}

/// ctime(t)：等价 asctime(localtime(t))（POSIX；共用同一个静态缓冲）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ctime(t: *const time_t) -> *mut c_char {
    unsafe {
        let lt = localtime(t);
        if lt.is_null() {
            return core::ptr::null_mut();
        }
        asctime(lt)
    }
}

/// mktime(tm)：把本地时间结构转成 epoch 秒（POSIX）。
///
/// **诚实边界（S09）**：本系统没有时区数据（见 localtime 的说明），故「本地时间」即 UTC——
/// 在本系统上 mktime 与 timegm 是同一个函数。
///
/// **归一化**：POSIX 允许/要求 mktime 归一化越界字段。下面用的是线性公式（Howard Hinnant 的
/// days_from_civil），故日/时/分/秒的越界自然进位；月先按 12 进位到年。归一化后的字段用
/// **localtime 反解**写回（同一事实来源，不另写一套反解）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mktime(t: *mut Tm) -> time_t {
    unsafe {
        if t.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        let mut year = (*t).tm_year + 1900;
        let mut mon = (*t).tm_mon;
        year += mon.div_euclid(12);
        mon = mon.rem_euclid(12);
        let m = mon + 1;
        let d = (*t).tm_mday;
        let h = (*t).tm_hour;
        let mi = (*t).tm_min;
        let s = (*t).tm_sec;
        let y = if m <= 2 { year - 1 } else { year };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era as i64 * 146097 + doe as i64 - 719468;
        let secs = days * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64;
        let tt = secs as time_t;
        let lt = localtime(&tt);
        if !lt.is_null() {
            *t = *lt;
        }
        tt
    }
}

/// \`time(tloc)\`：返回 Unix epoch 秒；tloc 非空则写入。
#[unsafe(no_mangle)]
pub extern "C" fn time(tloc: *mut time_t) -> time_t {
    match libsys::read_wall_clock() {
        Ok(wc) => {
            let epoch = wall_clock_to_epoch(&wc);
            if !tloc.is_null() {
                unsafe { *tloc = epoch; }
            }
            epoch
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// 把 WallClock（年月日时分秒）转换为 Unix epoch 秒（UTC，忽略时区）。
/// 算法：1970-01-01 起的累计天数 × 86400 + 当日秒。采用公历（proleptic）。
fn wall_clock_to_epoch(wc: &libsys::WallClock) -> time_t {
    let (y, m, d) = (wc.year as i64, wc.month as i64, wc.day as i64);
    let (hh, mi, ss) = (wc.hour as i64, wc.minute as i64, wc.second as i64);
    // 天数算法（civil_from_days 逆运算）：Zeller-like。
    let days = days_from_civil(y, m, d);
    days * 86400 + hh * 3600 + mi * 60 + ss
}

/// 从公历 (y,m,d) 计算自 1970-01-01 的天数（Howard Hinnant 算法，逆 days_from_civil）。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// \`clock()\`：返回进程运行时间（本实现 = 系统单调纳秒）。
#[unsafe(no_mangle)]
pub extern "C" fn clock() -> clock_t {
    // 单调时钟：内核 uptime_ms。返回纳秒（CLOCKS_PER_SEC=1e9）。
    match libsys::info(libsys::nr::INFO_BOOT_MS) {
        Ok(ms) => (ms * 1_000_000) as clock_t,
        Err(_) => -1,
    }
}

/// \`sleep(seconds)\`：睡眠指定秒数。
#[unsafe(no_mangle)]
pub extern "C" fn sleep(seconds: c_ulong) -> c_ulong {
    let ns = (seconds as u64).saturating_mul(1_000_000_000);
    match libsys::sleep(ns) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            seconds
        }
    }
}

/// \`usleep(usec)\`：睡眠指定微秒。
#[unsafe(no_mangle)]
pub extern "C" fn usleep(usec: c_ulong) -> c_int {
    let ns = (usec as u64).saturating_mul(1000);
    match libsys::sleep(ns) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`nanosleep(req, rem)\`：睡眠指定纳秒。
#[unsafe(no_mangle)]
pub extern "C" fn nanosleep(req: *const Timespec, _rem: *mut Timespec) -> c_int {
    if req.is_null() {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    let ts = unsafe { &*req };
    if ts.tv_sec < 0 || ts.tv_nsec < 0 || ts.tv_nsec >= 1_000_000_000 {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    let ns = (ts.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(ts.tv_nsec as u64);
    match libsys::sleep(ns) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`struct timespec\`（C 布局）。
/// `struct timeval`（C 布局，与 `libc/include/sys/time.h` 同一约定）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timeval {
    pub tv_sec: i64,
    pub tv_usec: i64,
}

/// `gettimeofday(tv, tz)`：BSD 风格取当前时间。
///
/// **`tv_usec` 恒为 0**：本系统墙钟来自内核 RTC（秒级分辨率）。不用单调时钟的亚秒部分去凑
/// 微秒——那是把两个不同时间源拼在一起，等于伪造数据（S09）。需要亚秒**间隔**测量的程序
/// 应使用 `clock()`（单调）。`tz` 参数按 POSIX 已废弃语义**忽略**。
#[unsafe(no_mangle)]
pub extern "C" fn gettimeofday(tv: *mut Timeval, _tz: *mut core::ffi::c_void) -> c_int {
    if tv.is_null() {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    match libsys::read_wall_clock() {
        Ok(wc) => {
            let epoch = wall_clock_to_epoch(&wc);
            unsafe {
                (*tv).tv_sec = epoch;
                (*tv).tv_usec = 0;
            }
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// `struct tm`（C 布局，字段顺序与 libc/include/time.h 一致）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Tm {
    pub tm_sec: i32,
    pub tm_min: i32,
    pub tm_hour: i32,
    pub tm_mday: i32,
    pub tm_mon: i32,
    pub tm_year: i32,
    pub tm_wday: i32,
    pub tm_yday: i32,
    pub tm_isdst: i32,
}

/// `gmtime`/`localtime` 共用的静态结果（POSIX 允许，并明确说可能被后续调用覆盖）。
static mut TM_BUF: Tm = Tm {
    tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 70,
    tm_wday: 4, tm_yday: 0, tm_isdst: 0,
};

/// 从"自 1970-01-01 的天数"求公历 (y, m, d)（Howard Hinnant 算法，与 days_from_civil 互逆）。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `gmtime(t)`：epoch 秒 → UTC 日历时间（静态存储）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gmtime(t: *const time_t) -> *mut Tm {
    if t.is_null() {
        set_errno(crate::errno::EINVAL);
        return core::ptr::null_mut();
    }
    let secs = unsafe { *t };
    // 向下取整除法（负数 epoch 也要正确）。
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let tm = unsafe { &mut *core::ptr::addr_of_mut!(TM_BUF) };
    tm.tm_sec = (rem % 60) as i32;
    tm.tm_min = ((rem / 60) % 60) as i32;
    tm.tm_hour = (rem / 3600) as i32;
    tm.tm_mday = d as i32;
    tm.tm_mon = (m - 1) as i32;
    tm.tm_year = (y - 1900) as i32;
    // 1970-01-01 是星期四（4）。
    tm.tm_wday = (days + 4).rem_euclid(7) as i32;
    tm.tm_yday = (days - days_from_civil(y, 1, 1)) as i32;
    tm.tm_isdst = 0;
    tm as *mut Tm
}

/// `localtime(t)`：**本系统无时区数据库**，故与 `gmtime` 完全相同（按 UTC 解释）。
///
/// **诚实边界（S39）**：POSIX 的 localtime 应受 TZ 影响；BORUIX 目前没有时区数据，
/// 故这里不做"假装有本地时区"的处理——直接用 UTC，并在此声明。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn localtime(t: *const time_t) -> *mut Tm {
    unsafe { gmtime(t) }
}

// ---------- strftime（3P6-2 第二波：由真实报错驱动补的第一项）----------
//
// 来路：tcc-on-boruix/tests/wave2.c 在 BORUIX 内用 tcc 编译时报
//   wave2.c:48: warning: implicit declaration of function 'strftime'
//   tcc: error: unresolved reference to 'strftime'
// 即**头文件没声明、库里也没有实现**。这不是预猜的清单项，是真实程序的真实报错。

/// 追加一段字节。返回 false 表示放不下（调用方按 POSIX 返回 0）。
///
/// 容量口径与 POSIX 一致：max 包含留给结尾 NUL 的那一个字节。
unsafe fn sf_put(s: *mut c_char, max: usize, out: &mut usize, bytes: &[u8]) -> bool {
    if *out + bytes.len() >= max {
        return false;
    }
    unsafe {
        for (i, b) in bytes.iter().enumerate() {
            *s.add(*out + i) = *b as c_char;
        }
    }
    *out += bytes.len();
    true
}

/// 追加一个十进制整数，pad 为左填充字符、width 为最小宽度（含负号）。
unsafe fn sf_num(
    s: *mut c_char,
    max: usize,
    out: &mut usize,
    val: i64,
    width: usize,
    pad: u8,
) -> bool {
    let mut buf = [0u8; 20];
    let neg = val < 0;
    let mut v = val.unsigned_abs();
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }
    let digits = n + if neg { 1 } else { 0 };
    let mut padn = width.saturating_sub(digits);
    while padn > 0 {
        if !unsafe { sf_put(s, max, out, &[pad]) } {
            return false;
        }
        padn -= 1;
    }
    if neg && !unsafe { sf_put(s, max, out, &[b'-']) } {
        return false;
    }
    while n > 0 {
        n -= 1;
        if !unsafe { sf_put(s, max, out, &[buf[n]]) } {
            return false;
        }
    }
    true
}

const SF_WDAY_ABBR: [&[u8]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
const SF_WDAY_FULL: [&[u8]; 7] = [
    b"Sunday", b"Monday", b"Tuesday", b"Wednesday", b"Thursday", b"Friday", b"Saturday",
];
const SF_MON_ABBR: [&[u8]; 12] = [
    b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov",
    b"Dec",
];
const SF_MON_FULL: [&[u8]; 12] = [
    b"January", b"February", b"March", b"April", b"May", b"June", b"July", b"August",
    b"September", b"October", b"November", b"December",
];

/// strftime 的主循环（把 fmt 渲染进 s）。%F/%T/%D/%R 递归调用自身。
unsafe fn sf_fmt(s: *mut c_char, max: usize, out: &mut usize, fmt: *const c_char, t: &Tm) -> bool {
    let mut i = 0usize;
    loop {
        let c = unsafe { *fmt.add(i) } as u8;
        if c == 0 {
            return true;
        }
        i += 1;
        if c != b'%' {
            if !unsafe { sf_put(s, max, out, &[c]) } {
                return false;
            }
            continue;
        }
        let mut d = unsafe { *fmt.add(i) } as u8;
        if d == 0 {
            return false;
        }
        i += 1;
        // POSIX 允许忽略 E/O 修饰符（用未修饰的等价形式）。
        if d == b'E' || d == b'O' {
            d = unsafe { *fmt.add(i) } as u8;
            if d == 0 {
                return false;
            }
            i += 1;
        }
        let wday = t.tm_wday.clamp(0, 6) as usize;
        let mon = t.tm_mon.clamp(0, 11) as usize;
        let year = (t.tm_year as i64) + 1900;
        let hour12 = {
            let h = (t.tm_hour as i64).rem_euclid(12);
            if h == 0 { 12 } else { h }
        };
        let ok = match d {
            b'Y' => unsafe { sf_num(s, max, out, year, 4, b'0') },
            b'y' => unsafe { sf_num(s, max, out, year.rem_euclid(100), 2, b'0') },
            b'C' => unsafe { sf_num(s, max, out, year.div_euclid(100), 2, b'0') },
            b'm' => unsafe { sf_num(s, max, out, (t.tm_mon as i64) + 1, 2, b'0') },
            b'd' => unsafe { sf_num(s, max, out, t.tm_mday as i64, 2, b'0') },
            b'e' => unsafe { sf_num(s, max, out, t.tm_mday as i64, 2, b' ') },
            b'H' => unsafe { sf_num(s, max, out, t.tm_hour as i64, 2, b'0') },
            b'I' => unsafe { sf_num(s, max, out, hour12, 2, b'0') },
            b'M' => unsafe { sf_num(s, max, out, t.tm_min as i64, 2, b'0') },
            b'S' => unsafe { sf_num(s, max, out, t.tm_sec as i64, 2, b'0') },
            b'j' => unsafe { sf_num(s, max, out, (t.tm_yday as i64) + 1, 3, b'0') },
            b'w' => unsafe { sf_num(s, max, out, t.tm_wday as i64, 1, b'0') },
            b'u' => unsafe {
                let u = t.tm_wday as i64;
                sf_num(s, max, out, if u == 0 { 7 } else { u }, 1, b'0')
            },
            b'a' => unsafe { sf_put(s, max, out, SF_WDAY_ABBR[wday]) },
            b'A' => unsafe { sf_put(s, max, out, SF_WDAY_FULL[wday]) },
            b'b' | b'h' => unsafe { sf_put(s, max, out, SF_MON_ABBR[mon]) },
            b'B' => unsafe { sf_put(s, max, out, SF_MON_FULL[mon]) },
            b'p' => unsafe { sf_put(s, max, out, if t.tm_hour < 12 { b"AM" } else { b"PM" }) },
            b'P' => unsafe { sf_put(s, max, out, if t.tm_hour < 12 { b"am" } else { b"pm" }) },
            // 本系统无时区数据库，恒为 UTC —— 这是事实陈述，不是占位。
            b'z' => unsafe { sf_put(s, max, out, b"+0000") },
            b'Z' => unsafe { sf_put(s, max, out, b"UTC") },
            b'F' => unsafe { sf_fmt(s, max, out, c"%Y-%m-%d".as_ptr(), t) },
            b'T' => unsafe { sf_fmt(s, max, out, c"%H:%M:%S".as_ptr(), t) },
            b'D' => unsafe { sf_fmt(s, max, out, c"%m/%d/%y".as_ptr(), t) },
            b'R' => unsafe { sf_fmt(s, max, out, c"%H:%M".as_ptr(), t) },
            b'n' => unsafe { sf_put(s, max, out, b"\n") },
            b't' => unsafe { sf_put(s, max, out, b"\t") },
            b'%' => unsafe { sf_put(s, max, out, b"%") },
            // POSIX 允许的兜底：原样输出，绝不静默丢弃。
            _ => unsafe { sf_put(s, max, out, &[b'%', d]) },
        };
        if !ok {
            return false;
        }
    }
}

/// strftime(s, max, format, tm)：按格式串把日历时间渲染进 s。
///
/// 返回写入的字节数（**不含**结尾 NUL）；结果放不下（含 NUL）或参数非法返回 0
/// （POSIX 语义：0 表示「未写入完整结果」）。
///
/// **支持集（诚实声明，S09）**：只实现真实程序常用的那一组——
/// %Y %y %C %m %d %e %H %I %M %S %p %P %j %w %u %a %A %b %h %B %F %T %D %R %n %t %%
/// 以及 %z/%Z（本系统无时区数据库，恒为 +0000/UTC）。未列出的说明符按 POSIX
/// 允许的方式**原样输出**（%X 输出 %X），绝不静默丢弃；E/O 修饰符按 POSIX 允许的
/// 方式忽略。**未实现**（明确声明，不是遗漏）：%c %x %X %U %W %V %G %g %s。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strftime(
    s: *mut c_char,
    max: size_t,
    format: *const c_char,
    tm: *const Tm,
) -> size_t {
    if s.is_null() || format.is_null() || tm.is_null() || max == 0 {
        return 0;
    }
    let t = unsafe { &*tm };
    let mut out = 0usize;
    if !unsafe { sf_fmt(s, max, &mut out, format, t) } {
        return 0;
    }
    if out >= max {
        return 0;
    }
    unsafe { *s.add(out) = 0 };
    out
}
