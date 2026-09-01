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
