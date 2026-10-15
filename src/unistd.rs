//! POSIX 系统调用封装（C ABI）：open/close/read/write/lseek/unlink 等。
//!
//! 真实数据链路（S06）：全部经 libsys 的 STREAM/VFS 域 syscall 落到内核。
//! 文件描述符为 libsys 返回的 u64 句柄。

use core::ffi::c_char;

use crate::ctypes::{c_int, size_t, ssize_t, c_void, c_uint};
use crate::errno::{set_errno, from_libsys, EINVAL};

/// open 标志（O_*）。
pub use crate::abi_generated::O_RDONLY;
pub use crate::abi_generated::O_WRONLY;
pub use crate::abi_generated::O_RDWR;
pub use crate::abi_generated::O_CREAT;
/// O_EXCL（3P6-2 第二波）：与 O_CREAT 同用时**独占创建**——文件已存在则
/// open 失败（EEXIST）。取值与 Linux 一致（0o200 = 0x80）。
pub use crate::abi_generated::O_EXCL;
/// O_CLOEXEC（3P4-3）：exec 时不继承该 fd。取值与 Linux 一致（0o2000000），
/// 与同处其余 O_* 的取值风格相同。
pub use crate::abi_generated::O_CLOEXEC;
pub use crate::abi_generated::O_TRUNC;
pub use crate::abi_generated::O_APPEND;

/// `open(path, flags, mode)`：打开文件，返回 fd 或 -1。
/// 进程级 umask（POSIX）。**默认 0**：本实现此前根本不看 mode，默认 0 可避免静默改变
/// 现有程序的权限；只有显式调用 `umask()` 之后才生效。
static UMASK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// 读当前 umask（供 `open`/`mkdir` 在建文件/目录时应用）。
pub fn umask_get() -> crate::ctypes::mode_t {
    UMASK.load(core::sync::atomic::Ordering::Relaxed)
}

/// `umask(mask)`：设置进程的文件创建掩码，返回**旧值**（POSIX）。
///
/// **诚实边界（S09）**：本内核的权限只有 owner 的 r/w/x 三个布尔（见 `sys/stat.h`），
/// 不存在 owner/group/other 矩阵 ⇒ 掩码里清除**组/其他**的那些位在本系统上**不可观测**
/// （`umask 022` 对 `0644` 毫无影响）；只有清除 **owner** 位（如 `0o200`）才可观测。
/// 这是内核权限模型的边界，不是本函数的近似。
#[unsafe(no_mangle)]
pub extern "C" fn umask(mask: crate::ctypes::mode_t) -> crate::ctypes::mode_t {
    UMASK.swap(mask & 0o777, core::sync::atomic::Ordering::Relaxed)
}

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
    // 权限：mode 按 **POSIX 位**解释（0644/0755…），经 `crate::mode` 的适配层折成内核的
    // 3 个布尔。此前直接取最低 3 位当 r/w/x，与调用方传的 0644 语义不通——
    // `0644 & 0b010 == 0` 会把「owner 可读写」建成**只读**（见 libc/src/mode.rs）。
    let (r, w, x) = crate::mode::perms_from_mode(mode, umask_get());
    let perm = libsys::Permissions {
        // 打开既有文件时，flags 侧的读/写意图照旧生效（O_RDWR 不该被 mode 挡住）。
        readable: r || read,
        writable: w || write,
        executable: x,
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

/// `close(fd)`：关闭 fd。
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
/// fd_pos_*）。这里把 oldfd 的位置**复制**到 newfd——与 close 清位置**对偶**（少了这一步，
/// 副本会从 0 开始读）。
///
/// **订正一处说错的注释（语义忠实性审查）**：原文写「POSIX 要求 dup2 的副本与原 fd **共享**
/// 同一文件偏移，故这里把位置复制过去」——**「共享」与「复制」不是一回事**，而代码做的是复制。
/// 真实情况分两种，如实写清：
///
/// - **未跟踪的 fd**（从未 lseek 过）：位置由**内核**维护在 `FileHandle.offset` 上，而
///   内核 dup 是 `Arc::clone`（**真共享**）⇒ 这一档**与 POSIX 一致** ✓；
/// - **已跟踪的 fd**（lseek 过）：位置在**本表的每-fd 槽位**里，dup2 只做一次性快照 ⇒
///   此后读其一**不会**推进另一个的位置，与 POSIX 的「共享同一打开文件描述」**不一致** ✗。
///
/// 这一档是**已知偏离**：彻底修好需要内核给一个「设置位置」或「按打开文件描述共享位置」的
/// 原语（内核只有 `STREAM_OFFSET_CURRENT` 的顺序推进与「显式偏移不推进」两种），
/// 属独立的内核变更。此处**如实登记**，不假装共享。
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

/// `read(fd, buf, count)`：读取字节。
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

/// `write(fd, buf, count)`：写入字节。
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
pub use crate::abi_generated::SEEK_SET;
pub use crate::abi_generated::SEEK_CUR;
pub use crate::abi_generated::SEEK_END;

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
    // POSIX：**不可定位**的 fd（管道/FIFO/字符设备/套接字）必须 **ESPIPE**，而且
    // **不得**把它转入「用户态维护位置」模式——否则此后 read/write 会走定位 I/O，
    // 被内核按节点真值拒绝（`Error::IllegalSeek`）：一个探测性的 lseek 就把管道用坏了。
    //
    // 判据**不靠 fd 号、也不靠类型位猜**，而是**问内核**：用**一字节定位读**探一次；
    // 返回 `IllegalSeek` 就是节点真值（该节点非顺序可定位）。
    //
    // **为什么是「一字节」而不是「零长」（实测逼出来的，不是推断）**：第一版用零长探针，
    // 理由是「内核先判 `is_seekable()`、再查长度」——**该推断被证伪**：内核 `sys_read`
    // 对 `len == 0` 有**前置短路**（`if len == 0 { return done(pack_ok(0)) }`，位置在
    // 管道判定与 `is_seekable` 判定**之前**），故零长探针什么都探不出来；机内用例当场
    // 报 `lseek(pipe, SEEK_SET) -> -1: FAIL`（passed=396 failed=3）。
    //
    // 一字节定位读是安全的：**显式偏移不推进任何位置**（内核的 `offset` 与 libc 的位置表
    // 都不动，这正是 `pread` 的语义），而对不可定位的 fd，内核在**任何数据被取走之前**
    // 就按节点真值返回 ESPIPE。
    //
    // **探针放在 `whence` 之前**（不是算完 `newpos` 之后）：否则 `SEEK_CUR` 会先撞上
    // 「未跟踪 ⇒ ENOTSUP」、`SEEK_END` 会先撞上 `fstat` 的 NotFound——而管道上这两个
    // whence 的 POSIX 答案都是 **ESPIPE**，不该被别的错误抢先。探针用偏移 0，
    // 对可定位文件无副作用。
    let mut probe = [0u8; 1];
    match libsys::pread(fd as u64, &mut probe, 0) {
        Err(libsys::Error::IllegalSeek) => {
            set_errno(crate::errno::ESPIPE);
            return -1;
        }
        // 内核 `sys_read` 对「fd 不存在」的**唯一**出口是 `InvalidParam`
        // （`syscall.rs:2634`/`2661`；本探针 len=1、offset=0，故 2702/2712 那两条
        // 用户地址溢出的出口不可达）。POSIX 要求坏 fd ⇒ **EBADF**；而**内核错误集里
        // 没有 EBADF**（`klib::error::Error` 无此项，见本轮登记），故这是 libc 唯一
        // 一处把「fd 不存在」译成 POSIX 码的地方——此前 `lseek` 对坏 fd **返回 0（假成功）**。
        Err(libsys::Error::InvalidParam) => {
            set_errno(crate::errno::EBADF);
            return -1;
        }
        // 其余错误（例如只写节点的读权限被拒）**不足以判定不可定位**：不据此拒绝，
        // 否则 `lseek` 在不可读的普通文件上会坏掉。残余边界见本轮报告。
        _ => {}
    }
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

/// `unlink(path)`：删除**非目录**的目录项（POSIX）。
///
/// **必须自己做类型判定**：内核的删除原语**同时支持删文件与空目录**
/// （`vfs::ramfs::unlink` 只看"目录是否为空"，**不看类型**，见 `ramfs.rs:352-376`）。
/// 所以不做这一层判定时，`unlink(<空目录>)` 会**真的把那个目录删掉**——而 POSIX 规定
/// 它是 `EPERM`（"The file named by path is a directory, and either the process does not
/// have appropriate privileges, or the implementation prohibits using unlink() on
/// directories"）。
///
/// 类型真值用 `lstat`（**不跟随**符号链接）：删一个指向目录的**符号链接**是合法的，
/// 该删的就是链接本身，所以这里要的是链接自己的类型。
///
/// **残余边界（S09）**：判定与删除是两次调用，之间存在 TOCTOU 窗口（把文件换成目录）。
/// 要彻底消除需要内核把"类型判定"并进删除原语（本轮登记，未做）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut st = core::mem::MaybeUninit::<stat>::uninit();
    if unsafe { crate::posix_batch2::lstat(path, st.as_mut_ptr()) } != 0 {
        // lstat 已置 errno（路径不存在 ⇒ ENOENT；前缀无 x 位 ⇒ EACCES）。
        return -1;
    }
    if unsafe { st.assume_init() }.st_mode & S_IFMT == S_IFDIR {
        set_errno(crate::errno::EPERM);
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

/// `readlink(path, buf, bufsiz)`：读软链接目标（POSIX）。返回**写入缓冲的字节数**
/// （**不含**终止 NUL；POSIX 也不要求 NUL 终止）。
///
/// **截断是 POSIX 规定的行为，不是「伪装完整」**。POSIX 原文：
/// > If the `buf` argument is not large enough to contain the link content, the first
/// > `bufsize` bytes shall be placed in `buf`.
/// > Upon successful completion, these functions shall return the count of bytes placed
/// > in the buffer.
///
/// `ERANGE` **不在** readlink 的错误表里。此前本实现写成「缓冲不足 ⇒ -1/ERANGE」并注释为
/// 「绝不把不完整目标伪装成完整」——**那个理由与 POSIX 相反**：POSIX 正是拿
/// 「返回值 == bufsiz」当**截断的可检测信号**（调用方据此知道缓冲不够），所以截断返回才是
/// 忠实实现；返回 ERANGE 会让**合法**的调用方拿到失败，而不是它有权拿到的那段前缀。
/// （语义忠实性审查第六轮改。）
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
    // POSIX：缓冲不足 ⇒ **截断**到 bufsiz 字节并返回 bufsiz（见函数文档）。
    let n = core::cmp::min(target.len(), bufsiz);
    unsafe {
        core::ptr::copy_nonoverlapping(target.as_ptr(), buf as *mut u8, n);
    }
    n as ssize_t
}

/// `_SC_NPROCESSORS_ONLN`：在线 CPU 数（取值与 Linux 一致）。
pub use crate::abi_generated::_SC_NPROCESSORS_ONLN;

/// `sysconf(name)`：系统配置查询（3P4-8c）。
///
/// 当前支持 `_SC_NPROCESSORS_ONLN`——数据来源**单一**：libsys 的
/// `info(INFO_CPU_COUNT)`（内核 `mm::cpu_count()` 经 sysfs `/system/info/cpu` 投影）。
///
/// **诚实边界（S09，语义忠实性审查第六轮订正）**：其余名字一律 -1/`EINVAL`。
/// POSIX 只允许对**未定义**的 name 返回 EINVAL；对**已定义**的 name（`_SC_OPEN_MAX`、
/// `_SC_ARG_MAX`、`_SC_PAGESIZE`…）本应返回真值。本系统多数没有对应数据源，故这里是
/// **已登记的偏离**（宁缺毋假：不编造返回值），不是「POSIX 允许」。要兑现需内核/环境补齐数据源。
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

/// `flock` 的操作位（取值与 glibc/Linux 一致；声明见 `<sys/file.h>`）。
pub use crate::abi_generated::LOCK_SH;
pub use crate::abi_generated::LOCK_EX;
pub use crate::abi_generated::LOCK_NB;
pub use crate::abi_generated::LOCK_UN;

/// `flock(fd, operation)`：BSD 风格 advisory 文件锁（声明见 `<sys/file.h>`）。
/// 成功返回 0，失败 -1 置 errno。
///
/// **已实现**：`LOCK_SH`/`LOCK_EX`/`LOCK_UN` 三个操作位，映射到内核
/// `SYS_STREAM_LOCK` 的共享/独占/释放。
///
/// **诚实边界（S09，逐条）**：
/// - 内核锁表**不阻塞**（`vfs::flock::flock_lock` 冲突即返回 `Busy`）。故对
///   **不带 `LOCK_NB`** 的加锁请求，本函数**如实返回 `ENOTSUP`**——
///   "等到拿到锁"这一半本系统做不到；假装成功、或静默降级成非阻塞，都会让调用方
///   基于错误前提继续跑。要非阻塞语义请显式带 `LOCK_NB`。
/// - 带 `LOCK_NB` 且冲突 ⇒ `EWOULDBLOCK`（Linux 对 `flock` 的约定；
///   内核给的是 `Busy`，此处按本 API 的约定换码）。
/// - 锁按**调用者 uid** 登记（内核 `LockOwner { uid }`），不是按打开文件描述 ⇒
///   **同一 uid 的多个进程共享同一把锁**（与 Linux 的 per-open-file-description 不同）。
/// - 未定义的操作位如实 `EINVAL`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn flock(fd: c_int, operation: c_int) -> c_int {
    if operation & !(LOCK_SH | LOCK_EX | LOCK_UN | LOCK_NB) != 0 {
        set_errno(EINVAL);
        return -1;
    }
    let nb = operation & LOCK_NB != 0;
    let op = operation & !LOCK_NB;
    let cmd = match op {
        LOCK_SH => libsys::nr::FLOCK_CMD_SHARED,
        LOCK_EX => libsys::nr::FLOCK_CMD_EXCLUSIVE,
        LOCK_UN => libsys::nr::FLOCK_CMD_UNLOCK,
        // 操作位必须恰为三者之一（Linux 同样不允许 LOCK_SH|LOCK_EX 这类组合）。
        _ => {
            set_errno(EINVAL);
            return -1;
        }
    };
    if op != LOCK_UN && !nb {
        set_errno(crate::errno::ENOTSUP);
        return -1;
    }
    match libsys::flock(fd as u64, cmd) {
        Ok(_) => 0,
        Err(libsys::Error::Busy) => {
            set_errno(crate::errno::EWOULDBLOCK);
            -1
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `link(target, link_path)`：创建**硬链接**（POSIX，声明见 `<unistd.h>`）。成功 0，失败 -1 置 errno。
///
/// 与 `symlink` 本质不同：软链接造**新节点**（内容是目标路径串），硬链接让**同一个节点**
/// 多一个目录项 ⇒ 两条路径共享内容、`st_ino` 相同、`st_nlink` 计数。
///
/// **诚实边界（S09）**：目前只有 **RamFS** 实现了该原语（根文件系统）。ext2 侧需先能从
/// `Arc<dyn INode>` 拿回目标 inode 号（内核 `add_dir_entry` 已收 ino，缺的是跨 trait 边界的
/// 身份通道），故 ext2 上如实返回 `ENOTSUP`——**不近似**（复制内容不是硬链接）。
/// `ttyname(fd)`：返回该 fd 对应终端的**路径**（POSIX，声明见 `<unistd.h>`）。
///
/// **本实现只能忠实回答「不是终端」这一半**：非终端 ⇒ NULL 置 `ENOTTY`（POSIX）。
/// 是终端时**如实返回 NULL 置 `ENOTTY`**——因为本系统目前**拿不到节点的名字**（见下），
/// 而**编一个路径**（例如恒返回 `/devices/console`）会在 fd 属于别的终端时撒谎。
///
/// # 为什么本系统**没有**可以返回的路径（实测 + ADR 裁定，不是推断）
///
/// **本系统按设计没有 TTY。** [ADR-042](../../docs/adr/042-input-side-no-legacy-canonical-mode.md)
/// （ACCEPTED）明定「输入侧同样不背历史债，**canonical 模式与 termios 一概不做**」，
/// 其事实表记着「全内核无 `termios`/`tty`/行规程实现」；内核只有**输出**侧的 console sink
/// （`term` crate）。⇒ `ttyname` 想返回的那个「终端**设备**的路径」在本系统**不存在**。
///
/// 内核里唯一与「终端」有关的是 `INode::is_terminal()`——一个**节点级布尔真值**
/// （ADR-044 / J-TOKEN-A），**唯一用途是让 `isatty` 说真话**，不是 TTY。实测：
///
/// - 它为真的**只有** `vfs/src/stdio.rs` 的两个节点（`StdinNode` @190、stdout 节点 @280），
///   即 **fd 0/1/2 背后的匿名节点**；
/// - 那些节点**不在任何目录里**（不是 devfs 下的设备文件）⇒ **没有路径可命名**；
/// - 而 `/devices/console`（devfs `ConsoleNode`）**没有覆写 `is_terminal`** ⇒ 按内核自己的
///   真值是 **`false`**，**它不是终端**。
///
/// 所以「终端 ⇒ `/devices/console`」是一条**凭空的映射**（本文件早先的版本正是这么写的，
/// 已订正）。即便退一步用 `st_ino` 做身份比对也不成立：`INode::stable_id` 默认是 `self`
/// 地址，其文档明写「无状态视图节点（procfs/devfs/stdio）的 `lookup` 同样可能**每次新建
/// 实例**」⇒ `fstat` 与 `stat` 拿到两个不同实例，身份必然不等（实测 `ttyname(1)` 返回 NULL）。
/// 反向查找（遍历挂载树）也无效——那些匿名节点根本不在树里。
///
/// **⇒ 本项属「设计性不支持」**（与 ADR-042 同族），不是「缺个组件」。
/// 若将来真要 `ttyname`，前提是先有 ADR 层面的新裁定（即决定引入终端设备节点），
/// 而不是在 libc 里补一层转译。在此之前本函数如实失败，**不返回伪造路径**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ttyname(fd: c_int) -> *mut c_char {
    // 非终端：忠实（POSIX 语义 + errno 都对）。
    // 是终端：本系统尚无「节点自述名字」通道，如实失败而不是编一个路径。
    let _ = fd;
    set_errno(crate::errno::ENOTTY);
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn link(target: *const c_char, link_path: *const c_char) -> c_int {
    if target.is_null() || link_path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let t = match unsafe { crate::stdio::cstr_to_str(target) } {
        Some(s) => s,
        None => {
            set_errno(EINVAL);
            return -1;
        }
    };
    let l = match unsafe { crate::stdio::cstr_to_str(link_path) } {
        Some(s) => s,
        None => {
            set_errno(EINVAL);
            return -1;
        }
    };
    match libsys::link(&t, &l) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `fsync(fd)`：把数据冲刷到稳定存储（POSIX，声明见 `<unistd.h>`）。成功 0，失败 -1 置 errno。
///
/// **诚实边界（S09）**：本内核的块缓存是**每设备**一份、不是每文件一份 ⇒ 无法只冲刷
/// 「该 fd 所在设备」。实现冲刷**全部**设备缓存——语义上是 POSIX 要求的**超集**
/// （fsync 的承诺是「数据已在稳定存储上」，多冲不违反），代价是粒度粗。这不是近似。
///
/// **仍脏即失败**：内核回传「仍脏块数」，> 0 时如实返回 -1 置 `EIO`（与关机路径同一条判据）。
#[unsafe(no_mangle)]
pub extern "C" fn fsync(fd: c_int) -> c_int {
    if fd < 0 {
        set_errno(crate::errno::EBADF);
        return -1;
    }
    match libsys::fsync(fd as u64) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `fchmod(fd, mode)`：按 fd 改文件模式（POSIX，声明见 `<sys/stat.h>`）。成功 0，失败 -1 置 errno。
///
/// **为什么不复用路径版 `chmod`**：本系统**没有 fd→路径 解析**（全仓无 `/proc/self/fd` 一类入口，
/// 实测零命中），故必须由内核提供 fd 版动词 `SYS_STREAM_FCHMOD`（0x1B）。判定面与路径版一致。
#[unsafe(no_mangle)]
pub extern "C" fn fchmod(fd: c_int, mode: crate::ctypes::mode_t) -> c_int {
    if fd < 0 {
        set_errno(crate::errno::EBADF);
        return -1;
    }
    match libsys::fchmod(fd as u64, mode) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `fchown(fd, uid, gid)`：按 fd 改属主（POSIX，声明见 `<unistd.h>`）。成功 0，失败 -1 置 errno。
///
/// **易他主需 `CAP_SYSTEM`**（内核单点判定，与路径版 `chown` 同一判据）；`uid`/`gid` 传 `-1`
/// 表示**保持不变**（POSIX）——先 `fstat` 取当前值再复用同一内核动词，不在用户态另造判定。
#[unsafe(no_mangle)]
pub extern "C" fn fchown(
    fd: c_int,
    uid: crate::ctypes::uid_t,
    gid: crate::ctypes::gid_t,
) -> c_int {
    if fd < 0 {
        set_errno(crate::errno::EBADF);
        return -1;
    }
    let mut st: stat = unsafe { core::mem::zeroed() };
    if unsafe { fstat(fd, &mut st) } != 0 {
        return -1; // fstat 已置 errno
    }
    let new_uid = if uid == u32::MAX { st.st_uid } else { uid };
    let new_gid = if gid == u32::MAX { st.st_gid } else { gid };
    match libsys::fchown(fd as u64, new_uid, new_gid) {
        Ok(()) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `chdir(path)`：切换工作目录。
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

/// `getcwd(buf, size)`：读当前工作目录。
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
/// **失败语义（订正后，逐条如实）**：
///
/// - **fd 有效但不是终端** -> 返回 0 并置 **ENOTTY**（POSIX 要求）。此前**不置 errno**，
///   于是"stdout 被重定向到普通文件"这个最常见的情形下，调用方读 errno 会拿到**上一次的残留值**；
/// - **fd 缺失或管道端** -> 返回 0，errno 用**内核给的那个**（内核 `sys_fstat` 对二者都返回
///   `NotFound` ⇒ 这里是 **ENOENT**）。**诚实边界**：POSIX 要求这两种分别置 `EBADF`
///   / `ENOTTY`，而内核的 `fstat` **无法区分**"fd 不存在"与"fd 是管道"，故**兑现不了**；
///   要兑现需内核补一个能查管道的 fstat。此处不假装——原注释写"置 EBADF"，
///   **那句与代码实际行为不符**，一并订正。
/// **不伪造终端**。安全侧：宁可如实说"不是终端"。
#[unsafe(no_mangle)]
pub extern "C" fn isatty(fd: c_int) -> c_int {
    match libsys::fstat(fd as u64) {
        Ok(info) => {
            if isatty_from_info(&info) != 0 {
                1
            } else {
                // POSIX：fd **有效**但不是终端 -> 0 并置 **ENOTTY**。
                // 此前不置 errno，于是"stdout 被重定向到普通文件"这个最常见的情形下，
                // 调用方读 errno 会拿到**上一次的残留值**。
                set_errno(crate::errno::ENOTTY);
                0
            }
        }
        Err(e) => {
            // fd 缺失或**管道端**（内核 sys_fstat 只处理文件，二者同为 NotFound）
            // => errno 用内核给的那个（ENOENT）。
            // **诚实边界**：POSIX 要求这两种分别置 EBADF / ENOTTY，而内核 fstat
            // **无法区分**"fd 不存在"与"fd 是管道"，故兑现不了——要兑现需内核补一个
            // 能查管道的 fstat。此处不假装。
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
    let (r, w, x) = crate::mode::perms_from_mode(mode, umask_get());
    let perm = libsys::Permissions {
        readable: r,
        writable: w,
        executable: x,
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
/// 故 remove 复用同一内核原语——**恰好**就是 POSIX `remove` 要的语义（它等价于
/// 「是目录就 rmdir、否则 unlink」），故这里**不加**类型判定。
///
/// 与 `unlink`/`rmdir` 的关系（语义忠实性审查第八轮）：那两者是**各自那一半**，
/// 内核原语不区分 ⇒ 类型判定必须由它们自己补（见各自的文档）。
/// 返回 0 成功，-1 失败置 errno。
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
/// 来路（3P6-2 第二波「整项缺失」类，反向对账列出）。
///
/// **原注释有一处自相矛盾且会导致真缺陷**：它说「内核在非目录/非空目录上如实报错」，
/// 但内核的删除原语**不看类型**（`ramfs.rs:352-376` 只拒**非空**目录）——于是
/// `rmdir(<普通文件>)` 会**真的把那个文件删掉**，而 POSIX 规定它是 `ENOTDIR`
/// （"The path argument names a file that is not a directory"）。
/// 「非空目录」那一半内核确实管（`NotEmpty` ⇒ ENOTEMPTY）。
///
/// 故这里补上类型判定（用 `lstat`，不跟随符号链接：`rmdir(<指向目录的链接>)` 按 POSIX
/// 是 ENOTDIR，链接不是目录）。TOCTOU 残余边界同 `unlink`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut st = core::mem::MaybeUninit::<stat>::uninit();
    if unsafe { crate::posix_batch2::lstat(path, st.as_mut_ptr()) } != 0 {
        return -1; // lstat 已置 errno
    }
    if unsafe { st.assume_init() }.st_mode & S_IFMT != S_IFDIR {
        set_errno(crate::errno::ENOTDIR);
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
pub use crate::abi_generated::WNOHANG;

// ---------- 3P6-2 第二波（GCC 宿主侧构建驱动的一批） ----------

/// `_PC_*`：pathconf 的名字常量（取值与 Linux 一致）。
pub use crate::abi_generated::_PC_LINK_MAX;
pub use crate::abi_generated::_PC_MAX_CANON;
pub use crate::abi_generated::_PC_MAX_INPUT;
pub use crate::abi_generated::_PC_NAME_MAX;
pub use crate::abi_generated::_PC_PATH_MAX;
pub use crate::abi_generated::_PC_PIPE_BUF;

/// getpagesize()：本系统的页大小（4 KiB）。
///
/// 来路：GCC 宿主侧构建（libiberty）报 `call to undeclared function 'getpagesize'`。
/// **诚实边界**：本实现返回**编译期常量 4096**（Boruix 的页大小是 4 KiB，见 mm 的 PageSize），
/// 不查内核——若将来支持可变页大小，此处需改为查询。
#[unsafe(no_mangle)]
pub extern "C" fn getpagesize() -> c_int {
    crate::abi_generated::PAGE_SIZE
}

/// `pathconf(path, name)`：按名字返回路径相关限制。
///
/// 来路：GCC 宿主侧构建报 `call to undeclared function 'pathconf'` + `_PC_PATH_MAX` 未定义。
///
/// **每个值的真值来源（S09：逐项说明，不编造）**：
///
/// | 名字 | 值 | 来源 |
/// | --- | --- | --- |
/// | `_PC_PATH_MAX` | 4096 | 内核 `syscall::MAX_USER_PATH_BYTES`（超限即 `NameTooLong`） |
/// | `_PC_NAME_MAX` | 255 | VFS `mount::MAX_NAME_BYTES`（末段名字契约，全部文件系统共用） |
/// | `_PC_PIPE_BUF` | 4096 | 内核 `ipc::PIPE_CAPACITY`——单次搬运块大小。**披露**：这是"最大单次拷贝"，
///   不是"并发写者的原子性保证"（后者 POSIX 有要求，本内核未单独论证） |
/// | `_PC_LINK_MAX` | -1 | **无上限**：VFS 不设硬链接计数上限（RamFS 的 `nlink` 是 `u32`）。
///   POSIX 正是用「返回 -1 且**不**改 errno」表示无限制 |
///
/// **已移除的编造值（本轮审查抓出）**：`_PC_MAX_CANON`/`_PC_MAX_INPUT` 曾报 255——那是
/// **行规程（canonical mode）**的限制，而本系统按 ADR-042 没有 termios、没有 TTY（`ttyname`
/// 同理）。为一个不存在的子系统报一个数就是凭空发明；现按本函数自己的规则如实 `EINVAL`。
/// `_PC_LINK_MAX` 曾报 1 并写「无硬链接能力」——那句在 `link(2)` 落地后**已经不成立**。
///
/// 其余名字**如实返回 -1 并置 EINVAL**。`path` 仅作非空校验：本系统各文件系统共用同一套
/// VFS 契约（见 `_PC_NAME_MAX`/`_PC_PATH_MAX` 的来源），故不存在 per-文件系统的差异表。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pathconf(path: *const c_char, name: c_int) -> crate::ctypes::c_long {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match name {
        _PC_PATH_MAX => crate::abi_generated::PATH_MAX,
        _PC_NAME_MAX => crate::abi_generated::NAME_MAX,
        _PC_PIPE_BUF => crate::abi_generated::PIPE_BUF,
        // 无限制：POSIX 规定此时返回 -1 且**不得**改动 errno。
        _PC_LINK_MAX => -1,
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

pub use crate::abi_generated::F_DUPFD;
pub use crate::abi_generated::F_GETFD;
pub use crate::abi_generated::F_SETFD;
pub use crate::abi_generated::F_GETFL;
pub use crate::abi_generated::F_SETFL;
/// FD_CLOEXEC：fd 标志。**内核有 `FLAG_CLOEXEC`**（`vfs::file_handle` 位 7：`open` 时
/// 可设、`exec` 时**真的关闭**该 fd，见 `kernel/src/syscall.rs:3841`；`dup2` 按 POSIX
/// 清除副本的该位，见同文件 1816-1823）——但内核**没有**「查询/设置已打开 fd 的 fd 标志」
/// 的原语，故本文件的 `F_GETFD`/`F_SETFD` 兑现不了 POSIX 语义（见下方边界）。
pub use crate::abi_generated::FD_CLOEXEC;

/// `fcntl(fd, cmd, ...)`：fd 控制。
///
/// 真实支持（S06）：`F_DUPFD`——复制 fd 到 `>=arg` 的最低空闲槽，经内核
/// `dup2` 实现（用 `dup2(fd,fd)` 非破坏性探测 fd 是否占用，见内核 sys_dup2：
/// `old==new` 时仅校验存在性）。
///
/// 诚实边界（S09）：
/// - 内核不暴露访问模式/状态旗标查询（`F_GETFL`）→ ENOTSUP；
/// - **CLOEXEC 只能在 `open` 时设定**（`libsys::OpenFlags::cloexec`；`fopen` 的 `"e"`
///   后缀走的就是这条路），内核没有「查询/设置已打开 fd 标志」的原语 ⇒ `F_GETFD`
///   无法如实回答、`F_SETFD` 无法生效。**早先这里的注释写着「本内核无 CLOEXEC
///   概念」——那句是错的**：内核有该位、且 `exec` 真的执行它；缺的是**查询/设置原语**。
///   要兑现需内核补一个 fd-flags 原语（语义忠实性审查第五轮登记）；
/// - 无 fd 级锁（`F_GETLK/F_SETLK`）→ ENOTSUP。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, args: ...) -> c_int {
    let mut ap = core::mem::transmute::<_, core::ffi::VaList>(args);
    match cmd {
        F_DUPFD => {
            let arg = unsafe { ap.next_arg::<c_int>() };
            // 与 dup() 共用同一实现（单点）：dup(fd) 按 POSIX 就等价于 F_DUPFD 且 arg=0。
            return dupfd_from(fd, arg);
        }
        // 本系统无法查询已打开 fd 的标志（无原语）。恒 0 是**近似**：若该 fd 是以
        // CLOEXEC 打开的，POSIX 要求这里返回 FD_CLOEXEC —— 如实登记为偏离，不假装。
        // 修法在内核（补 fd-flags 查询原语），不在本层。
        F_GETFD => 0,
        F_SETFD => {
            let flags = unsafe { ap.next_arg::<c_int>() };
            if flags == 0 {
                // POSIX：清 FD_CLOEXEC。本系统无原语可清：对**未以 CLOEXEC 打开**的 fd
                // 「没有该位」与「清掉它」等价；对以 CLOEXEC 打开的 fd **不等价**——
                // 这里返回 0 是保守近似（调用方通常只是「确保可继承」），偏离已登记。
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

pub use crate::abi_generated::S_IFMT;
pub use crate::abi_generated::S_IFSOCK;
pub use crate::abi_generated::S_IFLNK;
pub use crate::abi_generated::S_IFREG;
pub use crate::abi_generated::S_IFBLK;
pub use crate::abi_generated::S_IFDIR;
pub use crate::abi_generated::S_IFCHR;
pub use crate::abi_generated::S_IFIFO;

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
        st_ino: info.ino,   // D.2 link：文件身份真值（此前硬编码 0）
        // 内核侧 nlink 是 u32（POSIX 的 nlink_t 亦是），这里提升到 struct stat 的 u64 字段。
        st_nlink: info.nlink as u64, // D.2 link：硬链接计数真值（此前硬编码 0）
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
            console_owner: 0,
            ino: 7, nlink: 1,
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
            console_owner: 0,
            ino: 42, nlink: 3,
        };
        assert_eq!(isatty_from_info(&file), 0, "redirected to a file -> 0 (was 1)");

        // 未知（0）也如实报 0——安全侧，绝不把未知当终端。
        let unknown = StatInfo { is_terminal: 0, ..tty };
        assert_eq!(isatty_from_info(&unknown), 0);
    }
    /// `pathconf` 只报**有真值**的名字：没有 TTY 就没有行规程限制，如实 EINVAL 而不是
    /// 发明一个数字（本轮审查抓出 `_PC_MAX_CANON`/`_PC_MAX_INPUT` 曾报 255）。
    #[test]
    fn pathconf_reports_only_backed_values() {
        let path = b"/\0".as_ptr() as *const crate::ctypes::c_char;
        unsafe {
            assert_eq!(super::pathconf(path, super::_PC_NAME_MAX), 255);
            assert_eq!(super::pathconf(path, super::_PC_PATH_MAX), 4096);
            // 无上限：POSIX 编码 = 返回 -1 且**不**改 errno。
            crate::errno::set_errno(0);
            assert_eq!(super::pathconf(path, super::_PC_LINK_MAX), -1);
            assert_eq!(crate::errno::errno(), 0, "a no-limit answer must not set errno");
            // 不存在的子系统（无 canonical 模式）：如实 EINVAL。
            crate::errno::set_errno(0);
            assert_eq!(super::pathconf(path, super::_PC_MAX_CANON), -1);
            assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
            crate::errno::set_errno(0);
            assert_eq!(super::pathconf(path, super::_PC_MAX_INPUT), -1);
            assert_eq!(crate::errno::errno(), crate::errno::EINVAL);
        }
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
            console_owner: 0,
            ino: 1234, nlink: 2,
        };
        let st: stat = stat_from_info(&info);
        assert_eq!(st.st_mode & 0o777, 0o640, "three-segment mode restore");
        assert_eq!(st.st_uid, 1000, "real owner uid (was hardcoded 0)");
        assert_eq!(st.st_gid, 50, "real owner gid (was hardcoded 0)");
        assert_eq!(st.st_size, 42);
        // ino/nlink 也必须来自数据源（此前这两处没投影，st_ino/st_nlink 恒 0）。
        assert_eq!(st.st_ino, 1234, "ino projected from StatInfo");
        assert_eq!(st.st_nlink, 2, "nlink projected from StatInfo");
        // zero-owner 对照：0 是真值投影（数据源变更），非旧硬编码残留。
        let zero = StatInfo { owner_uid: 0, owner_gid: 0, ..info };
        let st2: stat = stat_from_info(&zero);
        assert_eq!(st2.st_uid, 0);
        assert_eq!(st2.st_gid, 0);
    }
}
