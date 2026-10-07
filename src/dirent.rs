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

use crate::ctypes::{c_char, c_int, c_long, c_ushort, c_uchar, c_void, ino_t, off_t};
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
/// **按 POSIX 返回 `void`**（此前 Rust 侧返回 `c_int`、头文件也误声明为 `int`——与 POSIX 不符，
/// 本轮一并改正）。NULL 指针是**无操作**（POSIX 对该情形未定义，取最保守行为）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rewinddir(dirp: *mut DIR) {
    if dirp.is_null() {
        return;
    }
    unsafe { (*dirp).cursor = 0 };
}

/// `telldir(dirp)`：返回目录流的当前位置（POSIX）。
///
/// 返回值对调用方**不透明**——唯一保证是「可以传给同一流的 `seekdir`」。本系统的目录流是
/// **打开时的快照**（见模块文档），故位置就是**项序号**。失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn telldir(dirp: *mut DIR) -> c_long {
    if dirp.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    (*dirp).cursor as c_long
}

/// `seekdir(dirp, loc)`：把目录流位置设到 `loc`（应当来自先前的 `telldir`）。
///
/// **越界如实夹紧**到 `[0, 项数]`：POSIX 说 `loc` 应来自 `telldir`，越界行为未定义；
/// 夹紧而不是报错，是为了让「`rewinddir` 之后 `seekdir(telldir(dir))`」这类正常往返不受影响。
///
/// **诚实边界**：快照模型下，`seekdir` 到**打开之后才增删**的项没有意义（快照不反映后续变化，
/// 与 `readdir` 同一限制）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn seekdir(dirp: *mut DIR, loc: c_long) {
    if dirp.is_null() {
        return;
    }
    let d = unsafe { &mut *dirp };
    let n = d.entries.len();
    d.cursor = if loc < 0 { 0 } else { (loc as usize).min(n) };
}

/// `alphasort(a, b)`：`scandir` 的标准比较器——按 `d_name` 字典序（POSIX）。
///
/// **诚实边界**：POSIX 规定用 `strcoll`（受 locale 影响）；本系统**没有 locale 数据库**
/// （`strcoll` 已判定为不支持），故这里用 `strcmp`——在 C locale 下两者等价，
/// 而本系统只有 C locale。**如实声明，不假装用了 strcoll**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn alphasort(
    a: *const *const dirent,
    b: *const *const dirent,
) -> c_int {
    if a.is_null() || b.is_null() {
        return 0;
    }
    let da = unsafe { *a };
    let db = unsafe { *b };
    if da.is_null() || db.is_null() {
        return 0;
    }
    crate::string::strcmp(
        unsafe { (*da).d_name.as_ptr() },
        unsafe { (*db).d_name.as_ptr() },
    )
}

/// `scandir(dirp, namelist, filter, compar)`：把目录项读成**排序后的数组**（POSIX）。
///
/// 返回项数；`*namelist` 指向 `malloc` 的、以 NULL 结尾的 `struct dirent *` 数组，
/// **每个元素也是单独 `malloc` 的**（POSIX 契约：调用方要逐个 `free` 再 `free` 数组）。
/// 失败返回 -1 置 errno。
///
/// 要点：
///  - `filter` 非空时只收 `filter(e) != 0` 的项；
///  - `compar` 非空时用 `qsort` 排序（**复用**已有实现，S15 单点；不另写排序）；
///  - `readdir` 的存储会被下一次调用覆盖，故每一项都**深拷贝**；
///  - 任何分配失败都**释放已分配的**并返回 -1（不留半成品、不漏内存）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scandir(
    path: *const c_char,
    namelist: *mut *mut *mut dirent,
    filter: Option<unsafe extern "C" fn(*const dirent) -> c_int>,
    compar: Option<unsafe extern "C" fn(*const *const dirent, *const *const dirent) -> c_int>,
) -> c_int {
    if path.is_null() || namelist.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let d = unsafe { opendir(path) };
    if d.is_null() {
        return -1; // errno 已由 opendir 置好
    }
    let mut v: Vec<*mut dirent> = Vec::new();
    let size = core::mem::size_of::<dirent>();
    loop {
        let e = unsafe { readdir(d) };
        if e.is_null() {
            break;
        }
        if let Some(f) = filter {
            if unsafe { f(e) } == 0 {
                continue;
            }
        }
        let copy = crate::malloc::malloc(size) as *mut dirent;
        if copy.is_null() {
            for p in v.iter() {
                crate::malloc::free(*p as *mut u8);
            }
            let _ = unsafe { closedir(d) };
            set_errno(crate::errno::ENOMEM);
            return -1;
        }
        unsafe { core::ptr::copy_nonoverlapping(e as *const u8, copy as *mut u8, size) };
        v.push(copy);
    }
    let _ = unsafe { closedir(d) };
    if let Some(cmp) = compar {
        // `qsort` 的比较器签名是 `(*const c_void, *const c_void)`，而 `scandir` 的是
        // `(*const *const dirent, *const *const dirent)`。数组元素本身就是 `struct dirent *`，
        // 故 `qsort` 传进来的正是 `&v[i]`——两种签名在 ABI 上一致（两个指针参数），
        // 这里做一次显式转型（glibc 同样这么做）。
        let c: crate::stdlib::CmpFn = unsafe { core::mem::transmute(cmp) };
        unsafe {
            crate::stdlib::qsort(
                v.as_mut_ptr() as *mut c_void,
                v.len(),
                core::mem::size_of::<*mut dirent>(),
                c,
            )
        };
    }
    let n = v.len();
    let bytes = (n + 1) * core::mem::size_of::<*mut dirent>();
    let arr = crate::malloc::malloc(bytes) as *mut *mut dirent;
    if arr.is_null() {
        for p in v.iter() {
            crate::malloc::free(*p as *mut u8);
        }
        set_errno(crate::errno::ENOMEM);
        return -1;
    }
    for (i, p) in v.iter().enumerate() {
        unsafe { *arr.add(i) = *p };
    }
    unsafe { *arr.add(n) = core::ptr::null_mut() };
    unsafe { *namelist = arr };
    n as c_int
}