//! 时间函数（C ABI）：time / clock / sleep / clock_gettime 简化。
//!
//! 真实数据链路（S06）：
//! - \`time()\` 返回 Unix epoch 秒（wall clock），来自 SysFS \`/system/info/time\`
//!   （内核 RTC 直读 CMOS）。SysFS 不可用时返回 (time_t)-1 置 errno（S09）。
//! - \`clock()\` 返回单调纳秒（内核 uptime_ms），换算为 CLOCKS_PER_SEC 单位。
//! - \`sleep\` 经 libsys \`sleep(ns)\` 阻塞。

use crate::ctypes::{c_int, c_ulong};
use crate::errno::{set_errno, from_libsys};

/// \`time_t\`：秒级时间类型（64 位）。
pub type time_t = i64;
/// \`clock_t\`：时钟计数类型。
pub type clock_t = i64;

/// 每秒时钟滴答数（与 \`clock()\` 返回值换算；本实现用纳秒）。
pub const CLOCKS_PER_SEC: clock_t = 1_000_000_000;

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
