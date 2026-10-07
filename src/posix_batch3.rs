//! 3P6-2 第二波（第三批）：GCC 宿主侧构建驱动的一批 POSIX 入口。
//!
//! 纪律与 `posix_batch2.rs` 相同：每项先核实底层能力存在（不预猜）再写最薄包装，
//! 诚实边界逐项写在函数上。
//!
//! **本批的新增前提是内核 O_EXCL（open 标志 bit 8）落地**：`mkstemp`/`mkdtemp` 的
//! 原子性此前根本无从谈起——「先 stat 再 create」的两步走有 TOCTOU 窗口，两个进程会
//! 同时判定「不存在」然后互相覆盖临时文件。`docs/TODO/libc-posix-surface.md` 曾把
//! 这两项登记为「判定不支持（依赖内核能力）」，本批**撤回该判定并落地**。
//!
//! 未在本批落地的一项（诚实登记，不静默跳过）：`tmpfile()`。它需要一个**可读可写**
//! 的 `FILE`，而本 libc 的 `FmMode` 只有 Read/Write/Append 三个变体、没有读写双向；
//! 用 `fdopen(fd, "w+b")` 顶替会得到一个**只写**流（`parse_mode` 只看首字符），
//! 那是半接线。要么先给 `FmMode` 加双向变体并同步全部 10 处模式判定，要么不做——
//! 见 `docs/TODO/libc-posix-surface.md` 的 C1 记录。

use crate::ctypes::{c_char, c_int, c_long, c_void, off_t, size_t, ssize_t, uid_t, gid_t};
use crate::errno::{from_libsys, set_errno};

/// `pread(fd, buf, count, offset)`：显式定位读，**不改变**文件偏移（POSIX）。
///
/// 来路：3P6-2 第二波反向对账（`audit_posix_surface.py`）列出的「整项缺失」类。
/// **底层能力已核实**：`libsys::pread` 早已导出（走 `SYS_STREAM_READ` 的显式 offset 参数），
/// 内核侧是**无状态**定位读——不推进句柄偏移。
///
/// 与本 libc 位置表的关系：`read()` 只在 fd 被 `lseek` 转入「用户态维护位置」后才用
/// 定位读（见 unistd.rs 的 read）。`pread` **不碰**那张表，故调用前后 `read()` 的续读
/// 位置完全不变——这正是 POSIX 对 pread 的硬要求。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pread(fd: c_int, buf: *mut c_void, count: size_t, offset: off_t) -> ssize_t {
    if buf.is_null() {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    // POSIX：count == 0 时不碰内核、直接返回 0（与 read 同一决策，理由见 read）。
    if count == 0 {
        return 0;
    }
    if offset < 0 {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, count) };
    match libsys::pread(fd as u64, slice, offset as u64) {
        Ok(n) => n as ssize_t,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `pwrite(fd, buf, count, offset)`：显式定位写，**不改变**文件偏移（POSIX）。
///
/// 与 `pread` 同源：`libsys::pwrite` 早已导出，内核侧无状态定位写。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrite(fd: c_int, buf: *const c_void, count: size_t, offset: off_t) -> ssize_t {
    if buf.is_null() {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    if count == 0 {
        return 0;
    }
    if offset < 0 {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
    match libsys::pwrite(fd as u64, slice, offset as u64) {
        Ok(n) => n as ssize_t,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// 查询本进程身份（uid/gid/caps 三元组）。失败返回 None（**不**兜底 0——0 是 root 的合法 uid，
/// 兜底等于伪造身份，S09 不允许）。
fn identity_or_errno() -> Option<libsys::IdentityInfo> {
    match libsys::identity_query() {
        Ok(i) => Some(i),
        Err(e) => {
            set_errno(from_libsys(e));
            None
        }
    }
}

/// `setuid(uid)`：设置本进程身份的真实/有效 uid（POSIX）。
///
/// 来路：反向对账的「整项缺失」类。**底层能力已核实**：`libsys::identity_set(uid, gid, caps)`
/// 早已导出，内核按 `CAP_SYSTEM` 二分授权（ADR-040 §3.5 G1）：无 CAP_SYSTEM 只能降权或不变，
/// 否则内核如实返回 `PermissionDenied`。
///
/// **诚实边界（S09）**：
///  - 本内核只维护**一份** uid，没有 real/effective/saved 之分，故 `setuid` 一次即改全部；
///  - `identity_set` 是三元组接口，故这里取**当前** gid/caps 原样带回，只改 uid——
///    绝不用 0 顶替未知的 gid/caps（那会**提权**：凭空把 gid 变成 0）。
#[unsafe(no_mangle)]
pub extern "C" fn setuid(uid: uid_t) -> c_int {
    let cur = match identity_or_errno() {
        Some(i) => i,
        None => return -1,
    };
    match libsys::identity_set(uid, cur.gid, cur.caps) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `setgid(gid)`：设置本进程的 gid（POSIX）。语义与 `setuid` 对称（见其说明）。
#[unsafe(no_mangle)]
pub extern "C" fn setgid(gid: gid_t) -> c_int {
    let cur = match identity_or_errno() {
        Some(i) => i,
        None => return -1,
    };
    match libsys::identity_set(cur.uid, gid, cur.caps) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// mkstemp / mkdtemp：**原子**独占创建（O_EXCL 的正面用例）
// ---------------------------------------------------------------------------

/// 名字字符集：POSIX 只要求「字母与数字」，这里用 base62（与 glibc 同集合）。
const TMP_ALNUM: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// 名字被占用后的重试次数。62^6 ≈ 5.6e10 种组合，128 次全部撞名在真实系统上不可能；
/// 上限存在的意义是**不发散**（模板非法/文件系统只读等情形不会变成死循环）。
const TMP_MAX_TRIES: u32 = 128;

/// 模块私有 PRNG（xorshift64*）。
///
/// **为什么不复用 rand/random**：POSIX 未规定临时文件名的随机源，而共用全局 PRNG 会让
/// `mkstemp` 悄悄消耗调用方的 `rand()` 序列（可观测的副作用，不该有）。
///
/// **诚实边界**：这不是密码学随机源。POSIX 的 `mkstemp` 不要求不可预测——它要求的是
/// **原子性**（由内核 O_EXCL 提供），随机名只用来降低撞名概率。
fn tmp_rng() -> u64 {
    static mut STATE: u64 = 0;
    unsafe {
        if STATE == 0 {
            // 种子：内核单调时钟 + 本函数地址（加载位置差异）+ 常数混合。
            // 取到 0 时用固定非零常数兜底（xorshift 的 0 是不动点，会退化成常量名）。
            STATE = libsys::now()
                ^ (tmp_rng as usize as u64).rotate_left(17)
                ^ 0x9E37_79B9_7F4A_7C15;
            if STATE == 0 {
                STATE = 0x9E37_79B9_7F4A_7C15;
            }
        }
        STATE ^= STATE >> 12;
        STATE ^= STATE << 25;
        STATE ^= STATE >> 27;
        STATE.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// 把模板末尾 6 个字符换成随机 alnum（调用方保证位置合法）。
unsafe fn scramble_suffix(p: *mut u8, len: usize) {
    for i in 0..6usize {
        let r = (tmp_rng() % 62) as usize;
        unsafe { *p.add(len - 6 + i) = TMP_ALNUM[r] };
    }
}

/// 校验模板末尾 6 个字符都是 'X'（POSIX 对 mkstemp/mkdtemp 模板的唯一要求）。
unsafe fn suffix_is_xxxxxx(p: *const u8, len: usize) -> bool {
    if len < 6 {
        return false;
    }
    for i in 0..6usize {
        if unsafe { *p.add(len - 6 + i) } != b'X' {
            return false;
        }
    }
    true
}

/// 模板串长度（不含结尾 NUL）。模板由调用方给出，必须以 NUL 结尾。
unsafe fn tpl_len(t: *const c_char) -> usize {
    let mut n = 0usize;
    while unsafe { *(t as *const u8).add(n) } != 0 {
        n += 1;
    }
    n
}

/// `mkstemp(template)`：以**原子独占**方式创建唯一临时文件，返回其 fd（POSIX）。
///
/// 模板末尾必须是 6 个 'X'；成功时这 6 个字符被就地替换成随机字符。
///
/// 原子性来自内核 `O_EXCL`（open 标志 bit 8）：内核在 `sys_open` 里**同一 syscall、同一
/// 临界区**内完成「存在性判定 + 创建」，中途无可被抢占的窗口。用户态无法自行补出这一
/// 性质——「先 stat 再 create」的两步走必然有 TOCTOU，两个进程会互相覆盖临时文件。
///
/// 失败：模板不以 6 个 'X' 结尾 → EINVAL；重试耗尽（128 次全部撞名）→ EEXIST；
/// 其他内核错误原样上抛（不吞、不伪造成功）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkstemp(template: *mut c_char) -> c_int {
    if template.is_null() {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    let len = unsafe { tpl_len(template) };
    let p = template as *mut u8;
    if !unsafe { suffix_is_xxxxxx(p, len) } {
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    // O_RDWR|O_CREAT|O_EXCL, 0600 —— POSIX 规定的 mkstemp 打开方式。
    let flags = libsys::OpenFlags {
        read: true,
        write: true,
        create: true,
        truncate: false,
        append: false,
        directory: false,
        pipe: false,
        cloexec: false,
        exclusive: true,
    };
    // 权限 0600：本系统权限模型只保留 r/w/x 三个布尔（无组/其他位），
    // 故 0600 与 0700 在此同形；如实注释，不假装有 owner/group/other 之分。
    let perm = libsys::Permissions::read_write();
    let mut tries = 0u32;
    while tries < TMP_MAX_TRIES {
        unsafe { scramble_suffix(p, len) };
        let path = match unsafe { crate::stdio::cstr_to_str(template) } {
            Some(s) => s,
            None => {
                set_errno(crate::errno::EINVAL);
                return -1;
            }
        };
        match libsys::open(path, flags, perm) {
            Ok(fd) => return fd as c_int,
            Err(libsys::Error::AlreadyExists) => {
                // 名字被占用：换名重试。**重试与创建之间没有窗口**——判定与创建同在内核内完成。
                tries += 1;
            }
            Err(e) => {
                set_errno(from_libsys(e));
                return -1;
            }
        }
    }
    set_errno(crate::errno::EEXIST);
    -1
}

/// `mkdtemp(template)`：以**原子独占**方式创建唯一临时目录（POSIX）。
///
/// 成功返回 `template` 本身（POSIX），失败返回 NULL 并置 errno。
///
/// **诚实边界**：POSIX 规定 mkdtemp 用 0700。本系统权限模型只有 r/w/x 三个布尔
/// （无 owner/group/other 之分，见 sys/stat.h 的成文说明），0700 在此映射为 rwx。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdtemp(template: *mut c_char) -> *mut c_char {
    if template.is_null() {
        set_errno(crate::errno::EINVAL);
        return core::ptr::null_mut();
    }
    let len = unsafe { tpl_len(template) };
    let p = template as *mut u8;
    if !unsafe { suffix_is_xxxxxx(p, len) } {
        set_errno(crate::errno::EINVAL);
        return core::ptr::null_mut();
    }
    let mut tries = 0u32;
    while tries < TMP_MAX_TRIES {
        unsafe { scramble_suffix(p, len) };
        let path = match unsafe { crate::stdio::cstr_to_str(template) } {
            Some(s) => s,
            None => {
                set_errno(crate::errno::EINVAL);
                return core::ptr::null_mut();
            }
        };
        match libsys::mkdir(path, libsys::Permissions::all()) {
            Ok(()) => return template,
            Err(libsys::Error::AlreadyExists) => {
                tries += 1;
            }
            Err(e) => {
                set_errno(from_libsys(e));
                return core::ptr::null_mut();
            }
        }
    }
    set_errno(crate::errno::EEXIST);
    core::ptr::null_mut()
}

// c_long 在本模块暂未直接使用，但 c_char/c_int 等别名与全 crate 保持一致；
// 显式引用避免 unused import 噪声（本 crate 对未用导入是 warning 级）。
#[allow(dead_code)]
type _KeepImports = (c_long,);
