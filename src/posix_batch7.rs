//! libc 表面收口批（第 7 批）：把生成式审计报出的 MISSING 里**真正可实现**的几项补齐。
//!
//! 判据（与 `docs/TODO/libc-posix-surface.md` 的三类归类一致）：
//! - **可实现**：机制在本系统已存在，只差包装 ⇒ 本文件补上；
//! - **设计性永久不支持**：由 ADR 明确拒绝（如 `ioctl`、进程组/session）⇒ **正式登记**，不伪造返回值；
//! - **由别的模块提供**：如 `dl*` 由 rtld 提供 ⇒ 登记归属，不进 libc。

use core::ffi::{c_char, c_int};

/// `gets(s)`：读一行到 `s`（不含换行，末尾写 NUL）。
///
/// **为什么还要实现它**：`gets` 在 C11 已被移除，但 POSIX 表面清单仍列它，且老代码会调。
/// 本实现**不能**消除它固有的缓冲区溢出风险（那正是 POSIX 废弃它的原因），故照定义实现，
/// 并在头文件里明确标注危险——不因为「危险」就假装它不存在。
///
/// 返回 `s`；EOF 且未读到任何字符时返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gets(s: *mut c_char) -> *mut c_char {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    let mut p = s;
    loop {
        let c = unsafe { crate::stdio::getchar() };
        if c < 0 {
            if p == s {
                return core::ptr::null_mut();
            }
            break;
        }
        if c == b'\n' as c_int {
            break;
        }
        unsafe {
            *p = c as c_char;
            p = p.add(1);
        }
    }
    unsafe { *p = 0 };
    s
}

/// `confstr(name, buf, len)`：取配置定义字符串（POSIX `<unistd.h>`）。
///
/// 本系统只有 C locale、且是单一系统的 sysroot，故可定义的项有限。已定义：
/// - `_CS_PATH`(1)：命令搜索路径。**运行期算**，与 shell 的默认 `PATH` 同源：
///   都调用 `libsys::search_path()`（唯一构造点）。
///
///   **为什么不再是编译期常量**：卷标、有几块盘、`3p` 在哪块盘上，都是**部署事实**。
///   把 `/volumes/<某个卷标>/3p` 烧进二进制，等于假定用户用的就是作者那块盘——
///   真实运行下卷标可能不同、可能没有第二块盘、`3p` 也可能在系统盘上。那样烧出来的值
///   会让本函数**返回一个不存在的目录**，而没有任何地方报错（静默伪数据，S09）。
///
///   **失败行为**：扫不动 `/volumes` 或没有任何卷带 `3p` ⇒ 只返回 `PROGRAM_DIR`，
///   那是**真值**，不是降级编造。
/// - `_CS_GNU_LIBC_VERSION`(2)：`boruix 0.1.0`——**如实标注是 boruix 而非 glibc**，
///   不给一个会被误认为真 glibc 的字符串。
///
/// 未定义的 `name` ⇒ 返回 0 并置 `EINVAL`（POSIX 允许：无值时返回 0）。
/// 返回值为所需长度（含 NUL）；`buf` 为 NULL 或 `len == 0` 时只返回长度。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn confstr(name: c_int, buf: *mut c_char, len: usize) -> usize {
    // `_CS_PATH` 是运行期算出来的（见上方文档）；其余项是静态串。
    // 两条路径都**含结尾 NUL**——confstr 的返回值含 NUL，长度也按含 NUL 计。
    let mut owned: alloc::vec::Vec<u8> = match name {
        1 => {
            let mut v = libsys::search_path().into_bytes();
            v.push(0);
            v
        }
        2 => b"boruix 0.1.0\0".to_vec(),
        _ => {
            crate::errno::set_errno(crate::errno::EINVAL);
            return 0;
        }
    };
    let v: &[u8] = &owned;
    if !buf.is_null() && len > 0 {
        let n = core::cmp::min(v.len(), len);
        unsafe { core::ptr::copy_nonoverlapping(v.as_ptr() as *const c_char, buf, n) };
        if n == len {
            unsafe { *buf.add(len - 1) = 0 };
        }
    }
    v.len()
}

/// `nl_langinfo(item)`：取当前 locale 的本地化信息（POSIX `<langinfo.h>`）。
///
/// 本系统只有 C locale（无 locale 数据库，`setlocale` 已判不支持）⇒ 返回 C locale 的值。
///
/// **诚实边界（S09）**：**POSIX 并未把 C locale 的每一项都钉死**——`CODESET` 的实现取值就是
/// 各系统自定（glibc 用 `ANSI_X3.4-1968`、FreeBSD 用 `US-ASCII`）。故本文件里的取值是
/// **本实现的 C locale 取值**，逐项如实，但**不声称**与 glibc/BSD 逐字节相同：
/// 日期/时间格式与 AM/PM、星期与月份名（含缩写）、`RADIXCHAR`/`THOUSEP`/`YESEXPR`/`NOEXPR`
/// 按 POSIX 的 C locale 定义；**`CODESET` 取 `ASCII`**——它如实描述本系统（纯 ASCII、
/// 无字符集子系统），但**不是** glibc/BSD 的那个名字。
///
/// （本项初版此处写的是「**规范值**（POSIX **明确规定**）」——那是**过度声明**；
/// 项目所有者质疑 `ttyname` 的同类问题后全面复查时订正。）
///
/// **编号来源是实测**：从 `envfiles/msys2/.../include/langinfo.h` 的枚举逐项解析得到
/// （0=CODESET、1..6=D_T_FMT..PM_STR、7..13=DAY_1..7、14..20=ABDAY、21..32=MON、
/// 33..44=ABMON、45..49=ERA/ALT_DIGITS、50=RADIXCHAR、51=THOUSEP、52=YESEXPR、
/// 53=NOEXPR、54=YESSTR、55=NOSTR、56=CRNCYSTR）——不靠记忆。
///
/// **诚实边界**：45..49（ERA/ERA_*/ALT_DIGITS）、51（THOUSEP）、54/55（YESSTR/NOSTR，已废弃）、
/// 56（CRNCYSTR）在 C locale 下确实无值 ⇒ 返回空串，这是 POSIX 的表示而非缺实现。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nl_langinfo(item: c_int) -> *mut c_char {
    const EMPTY: &[u8] = b"\0";
    const CODESET: &[u8] = b"ASCII\0";
    const D_T_FMT: &[u8] = b"%a %b %e %H:%M:%S %Y\0";
    const D_FMT: &[u8] = b"%m/%d/%y\0";
    const T_FMT: &[u8] = b"%H:%M:%S\0";
    const AM_STR: &[u8] = b"AM\0";
    const PM_STR: &[u8] = b"PM\0";
    const DAY: [&[u8]; 7] = [b"Sunday\0", b"Monday\0", b"Tuesday\0", b"Wednesday\0", b"Thursday\0", b"Friday\0", b"Saturday\0"];
    const ABDAY: [&[u8]; 7] = [b"Sun\0", b"Mon\0", b"Tue\0", b"Wed\0", b"Thu\0", b"Fri\0", b"Sat\0"];
    const MON: [&[u8]; 12] = [b"January\0", b"February\0", b"March\0", b"April\0", b"May\0", b"June\0", b"July\0", b"August\0", b"September\0", b"October\0", b"November\0", b"December\0"];
    const ABMON: [&[u8]; 12] = [b"Jan\0", b"Feb\0", b"Mar\0", b"Apr\0", b"May\0", b"Jun\0", b"Jul\0", b"Aug\0", b"Sep\0", b"Oct\0", b"Nov\0", b"Dec\0"];
    const RADIXCHAR: &[u8] = b".\0";
    const YESEXPR: &[u8] = b"^[yY]\0";
    const NOEXPR: &[u8] = b"^[nN]\0";
    let s: &[u8] = match item {
        0 => CODESET,
        1 => D_T_FMT,
        2 => D_FMT,
        // T_FMT_AMPM(4) 在 C locale 下与 T_FMT(3) 同值。
        3 | 4 => T_FMT,
        5 => AM_STR,
        6 => PM_STR,
        7..=13 => DAY[(item - 7) as usize],
        14..=20 => ABDAY[(item - 14) as usize],
        21..=32 => MON[(item - 21) as usize],
        33..=44 => ABMON[(item - 33) as usize],
        50 => RADIXCHAR,
        52 => YESEXPR,
        53 => NOEXPR,
        _ => EMPTY,
    };
    s.as_ptr() as *mut c_char
}
