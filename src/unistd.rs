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
    match libsys::close(fd as u64) {
        Ok(_) => 0,
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
    let slice = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, count) };
    match libsys::read(fd as u64, slice) {
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
    match libsys::write(fd as u64, slice) {
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

/// \`lseek(fd, offset, whence)\`：定位。
///
/// 内核 STREAM read/write 支持绝对偏移（pread/pwrite 语义）与顺序
/// （STREAM_OFFSET_CURRENT）。本实现用 libsys pread/pwrite 的绝对偏移表达
/// SEEK_SET；SEEK_CUR/SEEK_END 因内核不暴露当前位置而**如实返回 ENOTSUP**
/// （S09，不伪造）。
#[unsafe(no_mangle)]
pub extern "C" fn lseek(_fd: c_int, offset: crate::ctypes::c_long, whence: c_int) -> crate::ctypes::c_long {
    match whence {
        SEEK_SET => offset,
        _ => {
            // 内核无当前位置/文件末尾查询原语：如实不支持。
            set_errno(crate::errno::ENOTSUP);
            -1
        }
    }
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

/// \`isatty(fd)\`：fd 是否终端。本实现如实：stdin/out/err 视为终端。
#[unsafe(no_mangle)]
pub extern "C" fn isatty(fd: c_int) -> c_int {
    match fd {
        0 | 1 | 2 => 1,
        _ => 0,
    }
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

/// fcntl 命令常量（x86_64 Linux ABI）。
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
            let mut candidate: usize = if arg >= 0 { arg as usize } else { 0 };
            const MAX_FDS: usize = 1024;
            loop {
                if candidate >= MAX_FDS {
                    set_errno(crate::errno::EMFILE);
                    return -1;
                }
                // 探测 candidate 是否空闲：dup2(candidate,candidate) 恒非破坏。
                match libsys::dup2(candidate as u64, candidate as u64) {
                    Ok(_) => { candidate += 1; }
                    Err(libsys::Error::NotFound) => {
                        return match libsys::dup2(fd as u64, candidate as u64) {
                            Ok(_) => candidate as c_int,
                            Err(e) => { set_errno(from_libsys(e)); -1 }
                        };
                    }
                    Err(e) => { set_errno(from_libsys(e)); return -1; }
                }
            }
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
    use super::{stat_from_info, stat_perm_bits, stat};
    use libsys::StatInfo;

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
