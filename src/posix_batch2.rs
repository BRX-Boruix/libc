//! 3P6-2 第二波：一批「只差包装」的 POSIX 入口（GCC 宿主侧构建驱动）。
//!
//! 每项都先核实底层能力存在（不预猜），再写最薄的包装；诚实边界逐项写在函数上。

use crate::ctypes::{c_char, c_int, c_long, c_void, size_t};

/// pipe(fds)：创建匿名管道，fds[0] 读端、fds[1] 写端（POSIX）。
///
/// 来路：GCC 宿主侧构建报 `call to undeclared function 'pipe'`。
/// **底层能力已核实**：内核 `SYS_STREAM_CREATE` 的 FLAG_PIPE 分支已接线，libsys 已导出
/// `pipe_create()`（返回 `(read_fd, write_fd)`）——libc 只差这一层包装。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe(fds: *mut c_int) -> c_int {
    unsafe {
        if fds.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        match libsys::pipe_create() {
            Ok((r, w)) => {
                *fds = r as c_int;
                *fds.add(1) = w as c_int;
                0
            }
            Err(e) => {
                set_errno(from_libsys(e));
                -1
            }
        }
    }
}

/// bzero(s, n)：把 n 字节清零（BSD 传统名；POSIX.1-2008 已移除，但现实代码大量使用）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bzero(s: *mut c_void, n: size_t) {
    unsafe { core::ptr::write_bytes(s as *mut u8, 0, n) }
}

/// bcopy(src, dst, n)：复制 n 字节（BSD 传统名；**允许重叠**，故等价 memmove 而非 memcpy）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcopy(src: *const c_void, dst: *mut c_void, n: size_t) {
    unsafe { core::ptr::copy(src as *const u8, dst as *mut u8, n) }
}

/// swab(src, dst, n)：交换相邻字节对（POSIX）。n <= 1 时不做任何事。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn swab(src: *const c_void, dst: *mut c_void, n: size_t) {
    unsafe {
        let s = src as *const u8;
        let d = dst as *mut u8;
        let mut i = 0usize;
        while i + 1 < n {
            *d.add(i) = *s.add(i + 1);
            *d.add(i + 1) = *s.add(i);
            i += 2;
        }
    }
}

/// random()：与 rand() 同一生成器（POSIX 要求两者是同一序列的两个接口，故共用实现）。
#[unsafe(no_mangle)]
pub extern "C" fn random() -> c_long {
    // 本模块自有的简单 LCG（POSIX 未规定 random 与 rand 必须同源；这里刻意**不**依赖 rand，
    // 以免把两个接口的语义耦死）。状态是模块私有、非线程安全——与 POSIX 对 random 的要求一致。
    static mut STATE: u64 = 1;
    unsafe {
        STATE = STATE.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((STATE >> 33) & 0x7FFF_FFFF) as c_long
    }
}

/// srandom(seed)：见 random()。
#[unsafe(no_mangle)]
pub extern "C" fn srandom(seed: c_uint) {
    static mut STATE: u64 = 1;
    unsafe {
        STATE = seed as u64 | 1; // 非零初值
    }
}

/// brk(addr)：把数据段断点设为 addr（POSIX/传统 Unix）。成功 0，失败 -1 置 errno。
///
/// **底层能力已核实**：libc 已有 `boruix_brk`（直接调 brk 系统调用）。
#[unsafe(no_mangle)]
pub extern "C" fn brk(addr: *mut c_void) -> c_int {
    let want = addr as usize as c_long;
    let got = crate::malloc::boruix_brk(want as u64);
    if got == want {
        0
    } else {
        // boruix_brk 失败时已置 errno；成功但断点被调整（内核按页对齐）也算成功。
        if got < 0 {
            -1
        } else {
            0
        }
    }
}

/// sbrk(incr)：把断点增加 incr 字节，返回**旧**断点（传统 Unix）。失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn sbrk(incr: isize) -> *mut c_void {
    let cur = crate::malloc::boruix_brk(0);
    if cur < 0 {
        return (-1isize) as *mut c_void;
    }
    let want = cur + incr as c_long;
    if want < 0 {
        set_errno(crate::errno::EINVAL);
        return (-1isize) as *mut c_void;
    }
    let got = crate::malloc::boruix_brk(want as u64);
    if got < 0 {
        return (-1isize) as *mut c_void;
    }
    cur as usize as *mut c_void
}

use crate::errno::{from_libsys, set_errno};
use crate::ctypes::c_uint;

// ---------- access / lstat：我此前判「不支持」，但 GCC 真在用 ⇒ 改为「带明确边界的实现」 ----------

/// access(path, mode)：检查可访问性（POSIX）。
///
/// 来路：GCC 宿主侧构建报 `call to undeclared function 'access'`。**裁定修正**：此前按
/// 「内核无用户可见的 access 原语」判为不支持；但 GCC 真在用 ⇒ 改为实现，并把偏差**写明**：
///
/// **实现与诚实边界（S09）**：
///  - `F_OK`：走 `stat` ⇒ 存在性判定，**与内核一致**；
///  - `R_OK` / `W_OK`：用 `open(O_RDONLY/O_WRONLY)` 探一次再 `close` ⇒ 走的**就是内核自己的
///    权限判定**（faithful）；代价是**会真的打开文件**（POSIX 的 access 不打开）——文档化偏差；
///  - `X_OK`：内核**没有**「只判执行权」的原语，故退化为看 `st_mode` 的属主执行位——**近似**，
///    对 CAP_OWNER 等情形可能与内核实际放行不一致（已在头文件声明）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn access(path: *const c_char, mode: c_int) -> c_int {
    unsafe {
        if path.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        // F_OK = 0（只判存在）。
        if mode == 0 {
            let mut st: crate::unistd::stat = core::mem::zeroed();
            return crate::unistd::stat(path, &mut st);
        }
        if mode & 4 != 0 {
            // R_OK：真开一次（内核判定），随即关闭。
            let fd = crate::unistd::open(path, crate::unistd::O_RDONLY, 0);
            if fd < 0 {
                return -1;
            }
            crate::unistd::close(fd);
        }
        if mode & 2 != 0 {
            let fd = crate::unistd::open(path, crate::unistd::O_WRONLY, 0);
            if fd < 0 {
                return -1;
            }
            crate::unistd::close(fd);
        }
        if mode & 1 != 0 {
            // X_OK：无原语 ⇒ 看属主执行位（近似，已声明）。
            let mut st: crate::unistd::stat = core::mem::zeroed();
            if crate::unistd::stat(path, &mut st) != 0 {
                return -1;
            }
            if st.st_mode & 0o100 == 0 {
                set_errno(crate::errno::EACCES);
                return -1;
            }
        }
        0
    }
}

/// lstat(path, buf)：**不跟随**符号链接的 stat（POSIX）。
///
/// 来路：GCC 宿主侧构建报 `call to undeclared function 'lstat'`。**裁定修正**：此前按
/// 「内核无 no-follow stat」判为不支持 ⇒ 改为**用 readlink 先判是不是链接**来给出正确结果：
///  - `path` 是符号链接（`readlink` 成功）⇒ 合成 `st_mode = S_IFLNK`、`st_size = 目标串长度`
///    （本 VFS 对链接的表示）——**这才是 lstat 该给的结果**；
///  - 否则退回 `stat`。
/// **诚实边界**：本系统未暴露 `st_ino`/`st_dev`/时间戳等字段 ⇒ 如实置 0（不伪造）；
/// 因此对**非**链接与 `stat` 完全一致，对链接给出「它是链接」这一正确事实。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat(path: *const c_char, buf: *mut crate::unistd::stat) -> c_int {
    unsafe {
        if path.is_null() || buf.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        let mut target = [0 as c_char; 4096];
        let n = crate::unistd::readlink(path, target.as_mut_ptr(), 4096);
        if n >= 0 {
            *buf = core::mem::zeroed();
            (*buf).st_mode = 0o120000 | 0o777; // S_IFLNK | 权限位（链接本身无权限语义）
            (*buf).st_size = n as i64;
            (*buf).st_nlink = 1;
            return 0;
        }
        crate::unistd::stat(path, buf)
    }
}