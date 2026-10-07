//! POSIX 系统调用封装（C ABI）：open/close/read/write/lseek/unlink 等。
//!
//! 真实数据链路（S06）：全部经 libsys 的 STREAM/VFS 域 syscall 落到内核。
//! 文件描述符为 libsys 返回的 u64 句柄。

use core::ffi::c_char;

use crate::ctypes::{c_int, size_t, ssize_t, c_void, c_uint};
use crate::errno::{set_errno, from_libsys, EINVAL};

/// open 标志（O_*）。
pub const O_RDONLY: c_int = 0;
pub const O_WRONLY: c_int = 1;
pub const O_RDWR: c_int = 2;
pub const O_CREAT: c_int = 0x40;
/// O_EXCL（3P6-2 第二波）：与 O_CREAT 同用时**独占创建**——文件已存在则
/// open 失败（EEXIST）。取值与 Linux 一致（0o200 = 0x80）。
pub const O_EXCL: c_int = 0x80;
/// O_CLOEXEC（3P4-3）：exec 时不继承该 fd。取值与 Linux 一致（0o2000000），
/// 与同处其余 O_* 的取值风格相同。
pub const O_CLOEXEC: c_int = 0x80000;
pub const O_TRUNC: c_int = 0x200;
pub const O_APPEND: c_int = 0x400;

/// \`open(path, flags, mode)\`：打开文件，返回 fd 或 -1。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_uint) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let path_str = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    // 映射 flags。
    let read = flags & 3 == O_RDONLY || flags & 3 == O_RDWR;
    let write = flags & 3 == O_WRONLY || flags & 3 == O_RDWR;
    let create = flags & O_CREAT != 0;
    let truncate = flags & O_TRUNC != 0;
    let append = flags & O_APPEND != 0;
    let oflags = libsys::OpenFlags {
        read, write, create, truncate, append,
        directory: false, pipe: false,
        // 3P4-3：把 C 侧 O_CLOEXEC 透传到内核的每-fd 标志（fd 打标后由 exec 过滤）。
        cloexec: flags & O_CLOEXEC != 0,
        // O_EXCL：独占创建（内核在 sys_open 内原子判定，见该处注释）。
        exclusive: flags & O_EXCL != 0,
    };
    // 权限：从 mode 取 r/w/x 位（本内核 Permissions 用最低 3 位）。
    let perm = libsys::Permissions {
        readable: (mode & 0b100) != 0 || read,
        writable: (mode & 0b010) != 0 || write,
        executable: (mode & 0b001) != 0,
        system_only: false,
    };
    match libsys::open(path_str, oflags, perm) {
        Ok(fd) => fd as c_int,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`close(fd)\`：关闭 fd。
#[unsafe(no_mangle)]
pub extern "C" fn close(fd: c_int) -> c_int {
    // 位置表也要清：否则 fd 号被复用时会带着上一个文件的位置。
    fd_pos_lock();
    fd_pos_clear(fd);
    fd_pos_unlock();
    match libsys::close(fd as u64) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// dup2(oldfd, newfd)：把 oldfd 复制到 newfd（POSIX）。
///
/// 来路（3P6-2 第二波，GCC 真实报错驱动，不是预猜）：make all-gcc 编到 libiberty 时报
///   ../../gcc-14.2.0/libiberty/filedescriptor.c:45:10:
///   error: call to undeclared function 'dup2'
/// 而**内核与 libsys 早就有这个能力**（SYS_STREAM_DUP -> sys_dup2、libsys::io::dup2）——
/// 缺的只是 libc 的 C 包装与 <unistd.h> 声明。
///
/// 位置表：本 libc 在用户态维护「每 fd 的文件位置」（内核的流式读写只认显式 offset，见
/// fd_pos_*）。POSIX 要求 dup2 的副本与原 fd **共享**同一文件偏移，故这里把 oldfd 的位置
/// 复制到 newfd——与 close 清位置**对偶**（少了这一步，副本会从 0 开始读）。
#[unsafe(no_mangle)]
pub extern "C" fn dup2(oldfd: c_int, newfd: c_int) -> c_int {
    if oldfd < 0 || newfd < 0 {
        set_errno(EINVAL);
        return -1;
    }
    match libsys::dup2(oldfd as u64, newfd as u64) {
        Ok(_) => {
            fd_pos_lock();
            match fd_pos_get(oldfd) {
                Some(v) => fd_pos_set(newfd, v),
                None => fd_pos_clear(newfd),
            }
            fd_pos_unlock();
            newfd
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`read(fd, buf, count)\`：读取字节。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    if buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    // POSIX：`count == 0` 时**不碰内核**，直接返回 0。
    //
    // 此前把零长读下发内核，内核按非法参数报错 -> 返回 -1。后果是真实的：
    // tcc 的 `load_data(fd, off, sh_size)` 在 `sh_size == 0` 的节上做零长读，
    // `full_read` 拿到 -1（而上游**丢弃**这个返回值），于是缓冲区内容未定义，
    // 随后被当成节表/字符串表使用，最终野指针崩溃（内核留证 fault_addr 是垃圾值）。
    if count == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, count) };
    // 已被 lseek 转入「用户态维护位置」的 fd 用**定位读**并推进位置；否则顺序读（内核维护）。
    fd_pos_lock();
    let tracked = fd_pos_get(fd);
    let r = match tracked {
        Some(p) if p >= 0 => libsys::pread(fd as u64, slice, p as u64),
        Some(_) => {
            // 位置为负：lseek 已拒绝，正常到不了这里；如实报 EINVAL，不拿它当偏移。
            fd_pos_unlock();
            set_errno(EINVAL);
            return -1;
        }
        None => libsys::read(fd as u64, slice),
    };
    if let (Some(p), Ok(n)) = (tracked, &r) {
        fd_pos_set(fd, p + *n as i64);
    }
    fd_pos_unlock();
    match r {
        Ok(n) => n as ssize_t,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`write(fd, buf, count)\`：写入字节。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    if buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let slice = unsafe { core::slice::from_raw_parts(buf as *const u8, count) };
    fd_pos_lock();
    let tracked = fd_pos_get(fd);
    let r = match tracked {
        Some(p) if p >= 0 => libsys::pwrite(fd as u64, slice, p as u64),
        Some(_) => {
            fd_pos_unlock();
            set_errno(EINVAL);
            return -1;
        }
        None => libsys::write(fd as u64, slice),
    };
    if let (Some(p), Ok(n)) = (tracked, &r) {
        fd_pos_set(fd, p + *n as i64);
    }
    fd_pos_unlock();
    match r {
        Ok(n) => n as ssize_t,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// lseek whence 常量。
pub const SEEK_SET: c_int = 0;
pub const SEEK_CUR: c_int = 1;
pub const SEEK_END: c_int = 2;

// ---------- 文件位置（POSIX `lseek` 的用户态实现）----------
//
// **内核没有 seek 系统调用**：`SYS_STREAM_READ/WRITE` 每次自带偏移，
// `STREAM_OFFSET_CURRENT` 表示顺序（位置由内核维护），其他值表示定位 I/O 且**不推进**位置。
// 所以 POSIX 的「文件位置」必须由用户态维护——就是这里。
//
// **关键设计取舍**：只有程序**调用过 `lseek`** 之后，该 fd 才转入「定位 I/O」模式；
// 没调用过的 fd 一切照旧（顺序读，内核维护位置）。这样既有程序的行为**逐字不变**，
// 而 POSIX 程序也能正确工作——典型例子是 tcc：先读 64 字节判对象类型、再 `lseek` 复位、
// 再从头读节表；复位若是空操作，后面每一次读都会错位（实测报 `invalid object file`）。
//
// 为什么必须按 fd 惰性切换：`read`/`write` 是所有程序的地基，全局改成定位 I/O 会波及每一个
// 程序；而且管道/终端**不可定位**（内核按节点属性拒绝定位 I/O）。

/// 可跟踪的 fd 上限（fd 是小的非负整数）。
const MAX_TRACKED_FD: usize = 64;

#[derive(Clone, Copy)]
struct FdPos {
    /// 该 fd 是否已转入「用户态维护位置」模式（即被 `lseek` 动过）。
    tracked: bool,
    pos: i64,
}

static mut FD_POS: [FdPos; MAX_TRACKED_FD] =
    [const { FdPos { tracked: false, pos: 0 } }; MAX_TRACKED_FD];

/// 位置表的锁：`read`/`write` 会被多线程程序调用（本系统有线程）。
static FD_POS_LOCK: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

fn fd_pos_lock() {
    use core::sync::atomic::Ordering;
    while FD_POS_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn fd_pos_unlock() {
    FD_POS_LOCK.store(false, core::sync::atomic::Ordering::Release);
}

/// 取该 fd 已跟踪的位置；未跟踪或越界返回 `None`。
fn fd_pos_get(fd: c_int) -> Option<i64> {
    if fd < 0 || fd as usize >= MAX_TRACKED_FD {
        return None;
    }
    let t = unsafe { &*core::ptr::addr_of!(FD_POS) };
    let e = t[fd as usize];
    if e.tracked { Some(e.pos) } else { None }
}

/// 记录位置——**同时把它标记为已跟踪**，这是「转入定位 I/O」的开关。
fn fd_pos_set(fd: c_int, p: i64) {
    if fd < 0 || fd as usize >= MAX_TRACKED_FD {
        return;
    }
    let t = unsafe { &mut *core::ptr::addr_of_mut!(FD_POS) };
    t[fd as usize] = FdPos { tracked: true, pos: p };
}

/// 清除位置（`close` 时调用；否则 fd 号被复用时会带着上一个文件的位置）。
fn fd_pos_clear(fd: c_int) {
    if fd < 0 || fd as usize >= MAX_TRACKED_FD {
        return;
    }
    let t = unsafe { &mut *core::ptr::addr_of_mut!(FD_POS) };
    t[fd as usize] = FdPos { tracked: false, pos: 0 };
}

/// `lseek(fd, offset, whence)`：定位（POSIX 三态齐备）。
///
/// **本系统没有 seek 系统调用**，故「文件位置」由本函数在用户态维护（见上面的位置表说明）：
/// - `SEEK_SET`：新位置 = `offset`；
/// - `SEEK_CUR`：新位置 = 当前位置 + `offset`（该 fd 必须已被跟踪过，否则 ENOTSUP）；
/// - `SEEK_END`：新位置 = 文件大小 + `offset`（大小经 `fstat` 取——本系统没有「位置」，
///   但**大小是可得的**）。
///
/// **一旦调用过本函数，该 fd 就转入「用户态维护位置」模式**：此后 `read`/`write` 走定位 I/O
/// 并推进位置。这是 POSIX 程序的必需语义——tcc 正是这样读对象文件的（先读 64 字节判类型、
/// 再复位、再从头读节表）；复位若是空操作，后续每次读都会错位（实测报 `invalid object file`，
/// 而盘上那份文件其实是完好的 ELF，见 tcc-on-boruix/boruix/probe_file.c）。
#[unsafe(no_mangle)]
pub extern "C" fn lseek(
    fd: c_int,
    offset: crate::ctypes::c_long,
    whence: c_int,
) -> crate::ctypes::c_long {
    let newpos: i64 = match whence {
        SEEK_SET => offset,
        SEEK_CUR => {
            // 注意：取位置只碰用户态表，不阻塞；syscall 一律在锁外做。
            fd_pos_lock();
            let cur = fd_pos_get(fd);
            fd_pos_unlock();
            match cur {
                Some(p) => p + offset,
                None => {
                    // 该 fd 从未被定位过，用户态不知道「当前位置」（内核那侧也不暴露）：如实不支持。
                    set_errno(crate::errno::ENOTSUP);
                    return -1;
                }
            }
        }
        SEEK_END => {
            let mut st = core::mem::MaybeUninit::<stat>::uninit();
            if unsafe { fstat(fd, st.as_mut_ptr()) } != 0 {
                return -1;
            }
            unsafe { st.assume_init() }.st_size + offset
        }
        _ => {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
    };
    if newpos < 0 {
        // POSIX：定位到负偏移是 EINVAL。
        set_errno(crate::errno::EINVAL);
        return -1;
    }
    fd_pos_lock();
    fd_pos_set(fd, newpos);
    fd_pos_unlock();
    newpos
}

/// \`unlink(path)\`：删除文件。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::unlink(p) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `execvp(file, argv)`：**本系统不支持——如实失败，不伪造**。
///
/// BORUIX **没有"替换当前进程映像"的系统调用**：`SYS_TASK_SPAWN`（0x31）只**派生**新进程
/// 并返回其 pid（见 libsys::process::exec_path）。而 POSIX `execvp` 的核心契约恰恰是
/// "成功则不返回、当前进程变成新程序"——这在本系统上**无法成立**。
///
/// 因此本实现**总是返回 -1**，但给出**有区分度**的 errno（不把"找不到"与"做不了"混为一谈）：
/// - 程序按 PATH 找不到 → `ENOENT`（这是**真实**的查找结果）；
/// - 找到了 → `ENOTSUP`（"找到了，但本系统做不了 exec"）。
///
/// **为什么不"派生+等待+退出"来近似**：那会**静默改变**进程语义——pid 变了、父进程与
/// 观察者的关系变了、调用方的其余线程与资源处理也不同。库函数不该悄悄换语义。
///
/// 另注：本 ABI 的"命令行"是**单个字符串**（shell 已剥首词），不是 argv 数组；
/// 即便将来有了真 exec，argv→cmdline 的拼接也需要先定成文契约。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execvp(
    file: *const c_char,
    argv: *const *const c_char,
) -> c_int {
    let _ = argv;
    if file.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let name = match unsafe { crate::stdio::cstr_to_str(file) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    // 含 `/` 的按路径直接查；否则按 PATH 逐目录查（这一步是**真实**的，不是形式）。
    let found = if name.contains('/') {
        libsys::stat(name).is_ok()
    } else {
        let path = unsafe { crate::stdlib::getenv(c"PATH".as_ptr() as *const c_char) };
        let mut hit = false;
        if !path.is_null() {
            if let Some(p) = unsafe { crate::stdio::cstr_to_str(path) } {
                for dir in p.split(':') {
                    let cand = alloc::format!("{}/{}", if dir.is_empty() { "." } else { dir }, name);
                    if libsys::stat(&cand).is_ok() {
                        hit = true;
                        break;
                    }
                }
            }
        }
        hit
    };
    // 找不到 → ENOENT（真实结果）；找到了 → ENOTSUP（本系统没有替换映像的 exec）。
    set_errno(if found { crate::errno::ENOTSUP } else { crate::errno::ENOENT });
    -1
}

/// symlink(target, link_path)：创建软链接（3P4-8）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn symlink(target: *const c_char, link_path: *const c_char) -> c_int {
    if target.is_null() || link_path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let t = match unsafe { crate::stdio::cstr_to_str(target) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    let l = match unsafe { crate::stdio::cstr_to_str(link_path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::symlink(t, l) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `realpath(path, resolved)`：把路径规范化为**绝对、无 `.`/`..`、符号链接已解析**的形式。
///
/// 语义（POSIX）：
/// - `resolved == NULL`：本函数用 `malloc` 分配缓冲区，**调用方负责 `free`**（tcc 正是这样用的）；
/// - 否则写入调用方提供的缓冲区（调用方须保证足够大，POSIX 的 PATH_MAX）。
/// 失败返回 NULL 并置 errno。
///
/// **实现是真的在做规范化**：逐段处理输入，遇到符号链接就用 `readlink` 取出目标并接回待处理队列，
/// 而不是把输入原样返回。链接层数有上限（40，与 Linux 的 MAXSYMLINKS 同量级），超限返回 ELOOP。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realpath(
    path: *const c_char,
    resolved: *mut c_char,
) -> *mut c_char {
    use alloc::vec::Vec;
    if path.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let src = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };

    // `out`：已确认的绝对路径（不含结尾斜杠，根目录除外）；`pend`：尚待处理的字节。
    let mut out: Vec<u8> = Vec::new();
    let mut pend: Vec<u8> = Vec::new();
    if !src.starts_with('/') {
        // 相对路径：先接上当前工作目录（getcwd 已是绝对路径）。
        let mut cwdbuf = [0u8; 4096];
        if unsafe { getcwd(cwdbuf.as_mut_ptr() as *mut c_char, cwdbuf.len()) }.is_null() {
            return core::ptr::null_mut();
        }
        let n = cwdbuf.iter().position(|&b| b == 0).unwrap_or(0);
        pend.extend_from_slice(&cwdbuf[..n]);
        pend.push(b'/');
    }
    pend.extend_from_slice(src.as_bytes());

    let mut links = 0usize;
    loop {
        // 取下一个组件（跳过空段，处理 "//"）。
        while pend.first() == Some(&b'/') {
            pend.remove(0);
        }
        if pend.is_empty() {
            break;
        }
        let end = pend.iter().position(|&b| b == b'/').unwrap_or(pend.len());
        let comp: Vec<u8> = pend.drain(..end).collect();
        let rest: Vec<u8> = core::mem::take(&mut pend);

        if comp == b"." {
            pend = rest;
            continue;
        }
        if comp == b".." {
            // 回退一层（已在根则留在根）。
            while let Some(&b) = out.last() {
                out.pop();
                if b == b'/' {
                    break;
                }
            }
            if out.is_empty() {
                out.push(b'/');
            }
            pend = rest;
            continue;
        }

        // 普通组件：接到 out 后面，再看它是不是符号链接。
        let saved = out.len();
        if out.last() != Some(&b'/') {
            out.push(b'/');
        }
        out.extend_from_slice(&comp);

        let mut nbuf = [0u8; 4096];
        let n = unsafe { readlink(
            out.as_ptr() as *const c_char,
            nbuf.as_mut_ptr() as *mut c_char,
            nbuf.len(),
        ) };
        if n > 0 {
            // 是符号链接：把目标接回待处理队列，并撤销刚压入的组件。
            links += 1;
            if links > 40 {
                set_errno(crate::errno::ELOOP);
                return core::ptr::null_mut();
            }
            let target = &nbuf[..n as usize];
            out.truncate(saved);
            if target.first() == Some(&b'/') {
                // 绝对目标：从根重来。
                out.clear();
                out.push(b'/');
            }
            let mut merged: Vec<u8> = target.to_vec();
            merged.push(b'/');
            merged.extend_from_slice(&rest);
            pend = merged;
            continue;
        }
        pend = rest;
    }

    if out.is_empty() {
        out.push(b'/');
    }
    let n = out.len();
    // 输出：调用方缓冲或 malloc（POSIX：NULL 时由调用方 free）。
    let dst = if resolved.is_null() {
        crate::malloc::malloc(n + 1) as *mut c_char
    } else {
        resolved
    };
    if dst.is_null() {
        set_errno(crate::errno::ENOMEM);
        return core::ptr::null_mut();
    }
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), dst as *mut u8, n);
        *dst.add(n) = 0;
    }
    dst
}

/// `readlink(path, buf, bufsiz)`：读软链接目标（POSIX）。返回写入字节数（**不含**终止 NUL）。
///
/// **不截断**：缓冲不足时内核返回 NoSpace，此处如实转成 ERANGE——绝不把不完整目标
/// 伪装成完整（S09）。
///
/// **来路（3P6-2 第二波，反向对账列出）**：函数体**早就写在这里**，只是漏了
/// `#[unsafe(no_mangle)]` —— 于是 `libc.a` 里**没有**这个符号（`audit_posix_surface.py`
/// 的 PHANTOM 类：头文件声明了、库里没有，调用即链接失败）。补上属性即修复；
/// 这是「名字审计查不出」的一类缺陷：源码里有函数、库里有名字，两回事。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readlink(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> ssize_t {
    if path.is_null() || buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    let target = match libsys::readlink(p) {
        Ok(t) => t,
        Err(e) => {
            set_errno(from_libsys(e));
            return -1;
        }
    };
    if target.len() > bufsiz {
        set_errno(crate::errno::ERANGE);
        return -1;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(target.as_ptr(), buf as *mut u8, target.len());
    }
    target.len() as ssize_t
}

/// `_SC_NPROCESSORS_ONLN`：在线 CPU 数（取值与 Linux 一致）。
pub const _SC_NPROCESSORS_ONLN: c_int = 84;

/// `sysconf(name)`：系统配置查询（3P4-8c）。
///
/// 当前支持 `_SC_NPROCESSORS_ONLN`——数据来源**单一**：libsys 的
/// `info(INFO_CPU_COUNT)`（内核 `mm::cpu_count()` 经 sysfs `/system/info/cpu` 投影）。
/// 不支持的名字返回 -1 并置 `EINVAL`（POSIX 允许；**不编造**返回值）。
#[unsafe(no_mangle)]
pub extern "C" fn sysconf(name: c_int) -> crate::ctypes::c_long {
    match name {
        _SC_NPROCESSORS_ONLN => match libsys::info(libsys::nr::INFO_CPU_COUNT) {
            Ok(n) => n as crate::ctypes::c_long,
            Err(e) => {
                set_errno(from_libsys(e));
                -1
            }
        },
        _ => {
            set_errno(EINVAL);
            -1
        }
    }
}

/// ftruncate(fd, length)：按 fd 截断/扩展文件（3P4-8）。
#[unsafe(no_mangle)]
pub extern "C" fn ftruncate(fd: c_int, length: crate::ctypes::off_t) -> c_int {
    if length < 0 {
        set_errno(EINVAL);
        return -1;
    }
    match libsys::ftruncate(fd as u64, length as u64) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`chdir(path)\`：切换工作目录。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::chdir(p) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// \`getcwd(buf, size)\`：读当前工作目录。
#[unsafe(no_mangle)]
pub extern "C" fn getcwd(buf: *mut c_char, size: size_t) -> *mut c_char {
    if buf.is_null() || size == 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    match libsys::getcwd() {
        Ok(cwd) => {
            let bytes = cwd.as_bytes();
            if bytes.len() + 1 > size {
                set_errno(crate::errno::ERANGE);
                return core::ptr::null_mut();
            }
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
                *buf.add(bytes.len()) = 0;
            }
            buf
        }
        Err(e) => {
            set_errno(from_libsys(e));
            core::ptr::null_mut()
        }
    }
}

/// `isatty(fd)`：fd 是否终端。
///
/// **J-TOKEN-A ≡ T-ISATTY（ADR-044 §1.2）**：真值取自**节点自述**
/// （`INode::is_terminal()` → `StatInfo::is_terminal`），不再按 fd 号猜测。
///
/// **修复的缺陷**：旧实现 `match fd { 0|1|2 => 1, _ => 0 }` 注释自称
/// 「本实现如实」，实际「如实」的是"fd 号是不是 0/1/2"而非"这个 fd 是不是
/// 终端"——stdout 被重定向到普通文件或管道后**仍是 fd 1**，却依旧返回 1。
/// 那是硬编码猜测而非真值查询（违反 ADR-027 诚实契约与 S09/S10/S11）。
///
/// **失败语义**：fd 不存在即非终端 → 返回 0 并置 `EBADF`（POSIX 语义），
/// **不伪造终端**。安全侧：宁可如实说"不是终端"。
#[unsafe(no_mangle)]
pub extern "C" fn isatty(fd: c_int) -> c_int {
    match libsys::fstat(fd as u64) {
        Ok(info) => isatty_from_info(&info),
        Err(e) => {
            set_errno(from_libsys(e));
            0
        }
    }
}

/// `StatInfo` 真值 → `isatty` 返回值（**纯函数，宿主可测**）。
///
/// 与 `stat_from_info`/`stat_perm_bits` 同款分层：字节级 ABI 结构 → 语义
/// 值的转换单独成函数，从而可脱离内核/syscall 面直接断言（S23）。
pub(crate) fn isatty_from_info(info: &libsys::StatInfo) -> c_int {
    if info.is_terminal != 0 { 1 } else { 0 }
}

/// `mkdir(path, mode)`：创建目录。
///
/// `mode` 按 BORUIX 惯例取 r/w/x 位映射权限（readable/writable/executable）。
/// 返回 0 成功，-1 失败置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdir(path: *const c_char, mode: crate::ctypes::mode_t) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    let perm = libsys::Permissions {
        readable: (mode & 0b100) != 0,
        writable: (mode & 0b010) != 0,
        executable: (mode & 0b001) != 0,
        system_only: false,
    };
    match libsys::mkdir(p, perm) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `remove(path)`：删除文件或空目录（POSIX remove）。
///
/// 内核 `unlink` 同时支持删文件与空目录（sys_unlink → MountTable::unlink），
/// 故 remove 复用同一内核原语。返回 0 成功，-1 失败置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn remove(path: *const c_char) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::unlink(p) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// rmdir(path)：删除**空目录**（POSIX）。
///
/// 来路（3P6-2 第二波「整项缺失」类，反向对账列出）。**实现说明**：内核的 unlink 同时支持
/// 删文件与空目录（见 remove 的说明），POSIX 的 rmdir 只是「只接受目录」的那一半语义——
/// 内核在非目录/非空目录上如实报错，故这里复用同一调用，不另造路径。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_errno(EINVAL);
            return -1;
        }
    };
    match libsys::unlink(p) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// truncate(path, length)：把**路径**上的文件截断/扩展到指定长度（POSIX）。
///
/// 来路（3P6-2 第二波「整项缺失」类，反向对账列出）。**实现说明**：内核只有按 fd 的
/// ftruncate（libsys::ftruncate），故这里按 POSIX 允许的方式实现：open(O_WRONLY) + ftruncate
/// + close。POSIX 的 truncate 本来就要求调用方对文件有写权限，故语义一致。
/// close 可能覆盖 errno，故先保存 ftruncate 的 errno 再在失败时恢复。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn truncate(path: *const c_char, length: crate::ctypes::off_t) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let fd = unsafe { open(path, O_WRONLY, 0) };
    if fd < 0 {
        return -1;
    }
    let rc = ftruncate(fd, length);
    let saved = unsafe { *crate::errno::__errno_location() };
    close(fd);
    if rc != 0 {
        unsafe { *crate::errno::__errno_location() = saved; }
    }
    rc
}

/// fcntl 命令常量（x86_64 Linux ABI）。
/// waitpid 的 options（POSIX 归属 <sys/wait.h>；此处供 Rust 侧引用，单点定义）。
pub const WNOHANG: c_int = 1;

// ---------- 3P6-2 第二波（GCC 宿主侧构建驱动的一批） ----------

/// `_PC_*`：pathconf 的名字常量（取值与 Linux 一致）。
pub const _PC_LINK_MAX: c_int = 0;
pub const _PC_MAX_CANON: c_int = 1;
pub const _PC_MAX_INPUT: c_int = 2;
pub const _PC_NAME_MAX: c_int = 3;
pub const _PC_PATH_MAX: c_int = 4;
pub const _PC_PIPE_BUF: c_int = 5;

/// getpagesize()：本系统的页大小（4 KiB）。
///
/// 来路：GCC 宿主侧构建（libiberty）报 `call to undeclared function 'getpagesize'`。
/// **诚实边界**：本实现返回**编译期常量 4096**（Boruix 的页大小是 4 KiB，见 mm 的 PageSize），
/// 不查内核——若将来支持可变页大小，此处需改为查询。
#[unsafe(no_mangle)]
pub extern "C" fn getpagesize() -> c_int {
    4096
}

/// pathconf(path, name)：按名字返回路径相关限制。
///
/// 来路：GCC 宿主侧构建报 `call to undeclared function 'pathconf'` + `_PC_PATH_MAX` 未定义。
/// **诚实边界（S09）**：本系统没有 per-文件系统的限制表，故只对**确实有定义**的几个名字
/// 返回常量（路径/名字/管道缓冲上限），其余**如实返回 -1 并置 EINVAL**——不编造数值。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pathconf(path: *const c_char, name: c_int) -> crate::ctypes::c_long {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match name {
        _PC_PATH_MAX => 4096,
        _PC_NAME_MAX => 255,
        _PC_PIPE_BUF => 4096,
        _PC_MAX_CANON | _PC_MAX_INPUT => 255,
        // 无硬链接能力 ⇒ 如实报 1（只有一个名字指向该文件）；这是事实，不是占位。
        _PC_LINK_MAX => 1,
        _ => {
            set_errno(EINVAL);
            -1
        }
    }
}

/// mktemp(template)：把结尾的 "XXXXXX" 就地替换成一个唯一名，返回 template（失败返回空串）。
///
/// 来路：GCC 宿主侧构建报 `call to undeclared function 'mktemp'`。
/// **诚实边界（S09）**：mktemp 在 POSIX 里**已被标记为不安全**（有竞态，应改用 mkstemp）；
/// 本实现只做「用 pid + 单调计数器替换 XXXXXX」这一最小语义，**不声称它原子**——
/// 需要原子独占创建时应走 mkstemp（而那依赖内核的 O_EXCL，尚未支持）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mktemp(template: *mut c_char) -> *mut c_char {
    static COUNTER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    unsafe {
        if template.is_null() {
            return template;
        }
        let n = crate::string::strlen(template as *const c_char) as usize;
        if n < 6 {
            *template = 0;
            return template;
        }
        let tail = template.add(n - 6);
        for i in 0..6 {
            if *tail.add(i) != b'X' as c_char {
                *template = 0;
                return template;
            }
        }
        let pid = match libsys::getpid() { Ok(v) => v, Err(_) => 0 };
        let c = COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed) as u64;
        let mut v = (pid as u64).wrapping_mul(1_000_003).wrapping_add(c);
        const ALPHA: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        for i in 0..6 {
            *tail.add(i) = ALPHA[(v % 36) as usize] as c_char;
            v /= 36;
        }
        template
    }
}

pub const F_DUPFD: c_int = 0;
pub const F_GETFD: c_int = 1;
pub const F_SETFD: c_int = 2;
pub const F_GETFL: c_int = 3;
pub const F_SETFL: c_int = 4;
/// FD_CLOEXEC：fd 标志（本内核无 exec 关闭语义 → F_SETFD 恒拒绝）。
pub const FD_CLOEXEC: c_int = 1;

/// `fcntl(fd, cmd, ...)`：fd 控制。
///
/// 真实支持（S06）：`F_DUPFD`——复制 fd 到 `>=arg` 的最低空闲槽，经内核
/// `dup2` 实现（用 `dup2(fd,fd)` 非破坏性探测 fd 是否占用，见内核 sys_dup2：
/// `old==new` 时仅校验存在性）。
///
/// 诚实边界（S09）：内核不暴露访问模式/状态旗标查询（`F_GETFL`）、无
/// CLOEXEC（`F_SETFD`）、无 fd 级锁（`F_GETLK/F_SETLK`）→ 这些命令如实返回
/// ENOTSUP，不伪造。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, args: ...) -> c_int {
    let mut ap = core::mem::transmute::<_, core::ffi::VaList>(args);
    match cmd {
        F_DUPFD => {
            let arg = unsafe { ap.next_arg::<c_int>() };
            // 与 dup() 共用同一实现（单点）：dup(fd) 按 POSIX 就等价于 F_DUPFD 且 arg=0。
            return dupfd_from(fd, arg);
        }
        F_GETFD => 0, // 无 fd 标志（无 CLOEXEC 概念）。
        F_SETFD => {
            let flags = unsafe { ap.next_arg::<c_int>() };
            if flags == 0 {
                0
            } else {
                set_errno(crate::errno::ENOTSUP);
                -1
            }
        }
        _ => {
            set_errno(crate::errno::ENOTSUP);
            -1
        }
    }
}
/// 把 `fd` 复制到 `>= min` 的最低空闲槽（`fcntl(F_DUPFD)` 与 `dup()` 的**共同实现**）。
///
/// 探测手法：`dup2(n, n)` 在 `old == new` 时**仅校验存在性**（内核 sys_dup2 明确如此），
/// 故它是非破坏性的「该槽是否被占用」探针——这比 `fcntl(F_GETFD)` 更可靠（后者在本系统
/// 恒返回 0，无法区分占用与否）。
///
/// 成功后把用户态维护的「每 fd 文件位置」从 `fd` 复制到新 fd：POSIX 要求 dup/dup2/F_DUPFD
/// 的副本与原 fd **共享**同一文件偏移（与 close 清位置对偶）。
fn dupfd_from(fd: c_int, min: c_int) -> c_int {
    let mut candidate: usize = if min >= 0 { min as usize } else { 0 };
    const MAX_FDS: usize = 1024;
    loop {
        if candidate >= MAX_FDS {
            set_errno(crate::errno::EMFILE);
            return -1;
        }
        match libsys::dup2(candidate as u64, candidate as u64) {
            Ok(_) => candidate += 1,
            Err(libsys::Error::NotFound) => {
                let newfd = match libsys::dup2(fd as u64, candidate as u64) {
                    Ok(_) => candidate as c_int,
                    Err(e) => {
                        set_errno(from_libsys(e));
                        return -1;
                    }
                };
                fd_pos_lock();
                match fd_pos_get(fd) {
                    Some(v) => fd_pos_set(newfd, v),
                    None => fd_pos_clear(newfd),
                }
                fd_pos_unlock();
                return newfd;
            }
            Err(e) => {
                set_errno(from_libsys(e));
                return -1;
            }
        }
    }
}

/// dup(fd)：复制 fd 到**最低空闲**槽（POSIX）。
///
/// **来路（3P6-2 第二波，「整项缺失」类）**：`fcntl(F_DUPFD)` 里早已有这套非破坏探测逻辑，
/// 但 POSIX 的 `dup()` 本身既没实现也没声明（`libc/tools/audit_posix_surface.py` 的反向对账
/// 把它列了出来）。复用同一实现，故语义与 `fcntl(fd, F_DUPFD, 0)` 逐字一致。
#[unsafe(no_mangle)]
pub extern "C" fn dup(fd: c_int) -> c_int {
    dupfd_from(fd, 0)
}

// ---------- struct stat / stat / fstat / chmod / rename ----------

/// struct timespec (stat 时间戳用)。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// POSIX struct stat (x86_64 LP64 布局)。
///
/// 转业但诚实子集 (S09)：内核只暴露 r/w/x + system_only 四布尔，
/// 不存在 owner/group/other 矩阵。故：
/// - st_mode 的类型位 (S_IF*) 如实填入；
/// - 权限位：owner rwx 为真值，组/其他位镜像 owner (单用户擦平)。
/// - st_ino/st_dev 等内核未暴露的字段如实置 0，不伪造。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_nlink: u64,
    pub st_mode: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub __pad0: i32,
    pub st_rdev: u64,
    pub st_size: i64,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atim: timespec,
    pub st_mtim: timespec,
    pub st_ctim: timespec,
    pub __unused: [i64; 3],
}

pub const S_IFMT: u32 = 0o170000;
pub const S_IFSOCK: u32 = 0o140000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFIFO: u32 = 0o010000;

pub fn stat_type_bits(node_type: u32) -> u32 {
    match node_type {
        libsys::StatInfo::TYPE_DIR => S_IFDIR,
        libsys::StatInfo::TYPE_SYMLINK => S_IFLNK,
        libsys::StatInfo::TYPE_CHARDEV => S_IFCHR,
        libsys::StatInfo::TYPE_BLKDEV => S_IFBLK,
        libsys::StatInfo::TYPE_FIFO => S_IFIFO,
        libsys::StatInfo::TYPE_SOCKET => S_IFSOCK,
        _ => S_IFREG,
    }
}

/// wire classic 位 → POSIX mode 三段还原（A1-7：三段忠实，非任一位抹平）。
///
/// wire = classic 9 位直通（ADR-040 §2.5 裁决；旧 ≤0o7 披露位输入已由内核
/// `from_wire` 等值三段扩展，到这里恒为 classic 9 位形态）。owner/group/
/// other 逐段拼回 POSIX mode；门禁 bit9 非 mode 语义位，此函数不消费。
pub fn stat_perm_bits(perms: u32) -> u32 {
    perms & 0o777
}

fn stat_from_info(info: &libsys::StatInfo) -> stat {
    let mode = stat_type_bits(info.node_type) | stat_perm_bits(info.perms);
    stat {
        st_dev: 0,
        st_ino: 0,
        st_nlink: 0,
        st_mode: mode,
        // A1-7 归真（PRE-11「三层各自伪造」消解）：属主自 StatInfo 真值
        // 投影（A1-4 起内核 stat 通道如实报告节点属主），不再硬编码 0。
        st_uid: info.owner_uid,
        st_gid: info.owner_gid,
        __pad0: 0,
        st_rdev: 0,
        st_size: info.size as i64,
        st_blksize: 0,
        st_blocks: 0,
        st_atim: timespec { tv_sec: info.modified_time as i64, tv_nsec: 0 },
        st_mtim: timespec { tv_sec: info.modified_time as i64, tv_nsec: 0 },
        st_ctim: timespec { tv_sec: info.changed_time as i64, tv_nsec: 0 },
        __unused: [0; 3],
    }
}

/// stat(path, *mut stat)：读节点元数据 (POSIX stat，跟软链符)。
/// 返回 0 成功，-1 失败置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat(path: *const c_char, buf: *mut stat) -> c_int {
    if path.is_null() || buf.is_null() { set_errno(EINVAL); return -1; }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::stat(p) {
        Ok(info) => { unsafe { *buf = stat_from_info(&info); } 0 }
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// fstat(fd, *mut stat)：按 fd 读节点元数据。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, buf: *mut stat) -> c_int {
    if buf.is_null() { set_errno(EINVAL); return -1; }
    match libsys::fstat(fd as u64) {
        Ok(info) => { unsafe { *buf = stat_from_info(&info); } 0 }
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// chmod(path, mode)：设置节点权限（A1-7 归真：classic 9 位**三段忠实**
/// 透传——owner/group/other 各段独立编码，不再任一位抹平；PRE-3 消解）。
///
/// 系统门禁（bit9）不是 POSIX mode 语义位：POSIX chmod **不携带**门禁
/// 语义（门禁唯一写入门径仍是内核侧 set_permissions）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chmod(path: *const c_char, mode: crate::ctypes::mode_t) -> c_int {
    if path.is_null() { set_errno(EINVAL); return -1; }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    // classic 9 位直通（丢弃调用方可能误传的高位，如实按 POSIX 语义取 0o777 段）。
    match libsys::chmod(p, mode & 0o777) {
        Ok(_) => 0,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// chown(path, uid, gid)：易主（A1-7 落位，ADR-014 0x43 a4=2）。
///
/// 内核强制面：属主或 `CAP_OWNER`；**易他主**（新属主 ≠ 调用者）需
/// `CAP_SYSTEM`（POSIX chown 限制面，防权限赠予）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chown(path: *const c_char, uid: crate::ctypes::uid_t, gid: crate::ctypes::gid_t) -> c_int {
    if path.is_null() { set_errno(EINVAL); return -1; }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::chown(p, uid, gid) {
        Ok(_) => 0,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// rename(old, new)：同目录内重命名。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rename(oldpath: *const c_char, newpath: *const c_char) -> c_int {
    if oldpath.is_null() || newpath.is_null() { set_errno(EINVAL); return -1; }
    let old = match unsafe { crate::stdio::cstr_to_str(oldpath) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    let new = match unsafe { crate::stdio::cstr_to_str(newpath) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return -1; }
    };
    match libsys::rename(old, new) {
        Ok(_) => 0,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

// A1-7 / §3.2 #11：libc 归真宿主单测（纯逻辑层，无 syscall 面）。
// PRE-11「三层各自伪造」的回归锚：stat_perm_bits 三段忠实还原、
// stat_from_info 真属主投影（不再硬编码 0）。
#[cfg(test)]
mod a7_owner_truth_tests {
    use super::{stat_from_info, stat_perm_bits, stat, isatty_from_info};
    use libsys::StatInfo;


    /// **J-TOKEN-A ≡ T-ISATTY**（ADR-044 §1.2）：`isatty` 必须走**节点真值**
    /// （`StatInfo::is_terminal`），不得按 fd 号硬编码猜测。
    ///
    /// 此测锁定实现**读的是哪个字段**：同样的 fd 号、不同的真值 → 不同的
    /// 答案。硬编码 `fd ∈ {0,1,2} → 1` 无法通过本测。
    #[test]
    fn isatty_consults_node_truth_not_fd_number() {
        // 终端节点：真值 1 → isatty 报 1。
        let tty = StatInfo {
            node_type: StatInfo::TYPE_CHARDEV,
            size: 0,
            perms: 0o666,
            created_time: 0, modified_time: 0, changed_time: 0,
            owner_uid: 0, owner_gid: 0,
            is_terminal: 1,
        };
        assert_eq!(isatty_from_info(&tty), 1, "node says terminal -> 1");

        // **同 fd 号、普通文件**：真值 0 → isatty 必须报 0。
        // 这正是旧硬编码做不到的：stdout 重定向到文件后仍是 fd 1。
        let file = StatInfo {
            node_type: StatInfo::TYPE_FILE,
            size: 0,
            perms: 0o644,
            created_time: 0, modified_time: 0, changed_time: 0,
            owner_uid: 0, owner_gid: 0,
            is_terminal: 0,
        };
        assert_eq!(isatty_from_info(&file), 0, "redirected to a file -> 0 (was 1)");

        // 未知（0）也如实报 0——安全侧，绝不把未知当终端。
        let unknown = StatInfo { is_terminal: 0, ..tty };
        assert_eq!(isatty_from_info(&unknown), 0);
    }
    #[test]
    fn stat_perm_bits_restores_three_segments() {
        // 三段忠实：任意 classic 组合无损还原（不再任一位抹平）。
        assert_eq!(stat_perm_bits(0o644), 0o644);
        assert_eq!(stat_perm_bits(0o700), 0o700);
        assert_eq!(stat_perm_bits(0o077), 0o077);
        assert_eq!(stat_perm_bits(0o755), 0o755);
        assert_eq!(stat_perm_bits(0o111), 0o111);
        assert_eq!(stat_perm_bits(0o000), 0o000);
        // 门禁 bit9 非 mode 语义位：如实不进 POSIX mode。
        assert_eq!(stat_perm_bits(0o644 | (1 << 9)), 0o644);
    }

    #[test]
    fn stat_from_info_projects_real_owner() {
        let info = StatInfo {
            node_type: StatInfo::TYPE_FILE,
            size: 42,
            perms: 0o640,
            created_time: 0,
            modified_time: 100,
            changed_time: 200,
            owner_uid: 1000,
            owner_gid: 50,
            is_terminal: 0,
        };
        let st: stat = stat_from_info(&info);
        assert_eq!(st.st_mode & 0o777, 0o640, "three-segment mode restore");
        assert_eq!(st.st_uid, 1000, "real owner uid (was hardcoded 0)");
        assert_eq!(st.st_gid, 50, "real owner gid (was hardcoded 0)");
        assert_eq!(st.st_size, 42);
        // zero-owner 对照：0 是真值投影（数据源变更），非旧硬编码残留。
        let zero = StatInfo { owner_uid: 0, owner_gid: 0, ..info };
        let st2: stat = stat_from_info(&zero);
        assert_eq!(st2.st_uid, 0);
        assert_eq!(st2.st_gid, 0);
    }
}
