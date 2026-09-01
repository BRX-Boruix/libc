//! POSIX 目录流接口（C ABI）：opendir / readdir / closedir。
//!
//! 真实数据链路（S06）：`opendir` 经 `libsys::read_dir`（VFS 域 SYS_ENTRY_READ）
//! 一次性取回目录项快照（内核 `list_dir` → JSON），在打开时**快照**进本库持有的
//! `DIR`；`readdir` 逐项返回（每次覆盖同一 `struct dirent` 存储，符合 POSIX
//! "后续调用可覆盖上次返回值"的契约）；`closedir` 释放快照。
//!
//! 诚实边界（S09）：
//! - `d_ino`：内核目录项不含 inode 号 → 恒 0（不伪造）；
//! - `d_type`：内核 `type` 标签为 "dir"/"file" → 映射 `DT_DIR`/`DT_REG`，
//!   其余（特殊节点）→ `DT_UNKNOWN`；
//! - `opendir` 一次性快照，不反映打开后目录的变化（如实：无 dirfd 实时语义）。

use alloc::vec::Vec;

use crate::ctypes::{c_char, c_int, c_ushort, c_uchar, ino_t, off_t};
use crate::errno::{from_libsys, set_errno, EINVAL};

/// d_type 常量（POSIX / glibc 对齐）。
pub const DT_UNKNOWN: c_uchar = 0;
pub const DT_DIR: c_uchar = 4;
pub const DT_REG: c_uchar = 8;

/// `struct dirent`（BORUIX 精简 ABI，x86_64）。
#[repr(C)]
pub struct dirent {
    /// inode 号（内核未暴露 → 恒 0）。
    pub d_ino: ino_t,
    /// 目录内偏移（= 当前项序号）。
    pub d_off: off_t,
    /// 记录长度（`d_name` 实际占用）。
    pub d_reclen: c_ushort,
    /// 条目类型（DT_DIR/DT_REG/DT_UNKNOWN）。
    pub d_type: c_uchar,
    /// 名字（NUL 终止），固定 256 字节。
    pub d_name: [c_char; 256],
}

/// `DIR`：目录流状态（快照 + 游标 + 当前 dirent 存储）。
pub struct DIR {
    /// 打开时快照的目录项（name, type, size）。
    entries: Vec<libsys::DirEntry>,
    /// 下一个待返回项的索引。
    cursor: usize,
    /// `readdir` 返回的 `struct dirent` 存储（每次覆盖）。
    cur: dirent,
}

/// `opendir(path)`：打开目录流，返回 `DIR*` 或 NULL（置 errno）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut DIR {
    if path.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let p = match unsafe { crate::stdio::cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_errno(EINVAL);
            return core::ptr::null_mut();
        }
    };
    match libsys::read_dir(p) {
        Ok(entries) => {
            let dir = alloc::boxed::Box::new(DIR {
                entries,
                cursor: 0,
                cur: dirent {
                    d_ino: 0,
                    d_off: 0,
                    d_reclen: 0,
                    d_type: DT_UNKNOWN,
                    d_name: [0; 256],
                },
            });
            alloc::boxed::Box::into_raw(dir)
        }
        Err(e) => {
            set_errno(from_libsys(e));
            core::ptr::null_mut()
        }
    }
}

/// 把 libsys `DirEntry` 填入 `struct dirent`（覆盖 `dir.cur`）。
fn fill_dirent(dir: &mut DIR) {
    let e = &dir.entries[dir.cursor];
    // 名字拷贝进固定 256 缓冲（截断 + NUL 终止，防越界）。
    let mut name = [0i8; 256];
    let bytes = e.name.as_bytes();
    let n = bytes.len().min(255);
    for i in 0..n {
        name[i] = bytes[i] as i8;
    }
    name[n] = 0;
    let d_type = match e.node_type.as_str() {
        "dir" => DT_DIR,
        "file" => DT_REG,
        _ => DT_UNKNOWN,
    };
    dir.cur.d_ino = 0;
    dir.cur.d_off = dir.cursor as off_t;
    dir.cur.d_type = d_type;
    // d_reclen：dirent 从 d_name 起的字节数（不含首部字段）。
    dir.cur.d_reclen = (n + 1) as c_ushort;
    dir.cur.d_name = name;
}

/// `readdir(dirp)`：返回下一个目录项，或 NULL（到达末尾 / 出错）。
///
/// 返回指针指向 `DIR` 内存储，**下一次调用会覆盖**（POSIX 契约）。到达末尾
/// 返回 NULL（errno 不清，符合 POSIX）；出错返回 NULL 并置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir(dirp: *mut DIR) -> *mut dirent {
    if dirp.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let dir = unsafe { &mut *dirp };
    if dir.cursor >= dir.entries.len() {
        return core::ptr::null_mut();
    }
    fill_dirent(dir);
    dir.cursor += 1;
    &mut dir.cur as *mut dirent
}

/// `closedir(dirp)`：关闭目录流并释放快照。
///
/// 返回 0 成功，-1 失败置 errno（NULL 指针 → EINVAL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn closedir(dirp: *mut DIR) -> c_int {
    if dirp.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    // 释放 DIR（含 entries 快照）。
    unsafe {
        drop(alloc::boxed::Box::from_raw(dirp));
    }
    0
}

/// `rewinddir(dirp)`：把目录流游标重置到开头（POSIX）。
///
/// 返回 0 成功，-1 失败置 errno（NULL 指针 → EINVAL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rewinddir(dirp: *mut DIR) -> c_int {
    if dirp.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { (*dirp).cursor = 0 };
    0
}