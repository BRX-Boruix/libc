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
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}
