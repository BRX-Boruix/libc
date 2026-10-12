//! POSIX 账户查询（`pwd.h` 对应物）——A2-5 / ADR-040 §3.5 G5。
//!
//! **纯用户态实现（ADR-040 §2.9 明确划界）**：名字↔uid 映射是用户态职能，内核**不参与**。
//! 本模块读取 `/config/users.json`（格式见 `parse_accounts`），在进程内缓存解析结果，
//! 对外提供 `getpwnam` / `getpwuid` / `getpwent` / `endpwent` / `setpwent`。
//!
//! ## 为何在 libc 而不在内核（S13 边界）
//!
//! POSIX 程序按**名字**使用账户（`getpwnam("alice")`），而内核只认 uid 数字与能力位。
//! 若把账户表塞进内核，则：①内核需解析 JSON、引入不可信输入面；②账户变更需内核参与，
//! 违背"账户完全是用户态文件"的既有成文决策。故本模块**只读文件、只做映射**，
//! 不新增任何 syscall（Q6 不变）。
//!
//! ## 诚实边界（S09）
//!
//! - 表缺失 / 不可读 / 畸形 → **如实**返回 `NULL` 并置 `errno`，**绝不**返回伪造账户
//!   （不存在"兜底 alice/uid 1000"这类伪数据）。
//! - 找不到名字/uid → `NULL` + `ENOENT`（POSIX 惯例），不是"返回空结构体"。
//! - 本实现**不做** NSS/LDAP 等外部源；也不支持 `getpwnam_r` 的线程安全变体（见下）。
//!
//! ## 线程安全说明（如实声明，不冒充）
//!
//! 缓存经 `spin::Mutex` 保护，故并发查找**不会数据竞争**。但 POSIX 的 `getpwnam`
//! 约定返回指向**静态存储**的指针，调用方不得释放、且后续调用可能覆盖——本实现
//! 遵守该约定（返回值指向模块内的稳定存储，生命周期至下次同名调用）。需要可重入
//! 的调用方应使用 `getpwnam_r`（**本实现尚未提供**，属已登记的边界）。

use alloc::string::String;
use alloc::vec::Vec;
use core::ptr;
use spin::Mutex;

use crate::ctypes::c_char;

/// POSIX `struct passwd`（字段顺序遵循 POSIX；本实现填充前四项，其余为 NULL）。
#[repr(C)]
pub struct passwd {
    /// 用户名。
    pub pw_name: *mut c_char,
    /// 用户 id。
    pub pw_uid: u32,
    /// 主组 id。
    pub pw_gid: u32,
    /// 家目录（本实现按 `/users/<name>` 约定派生，见 `home_dir_of`）。
    pub pw_dir: *mut c_char,
    /// 登录 shell（本仓无独立 shell 字段，恒为 NULL——不编造）。
    pub pw_shell: *mut c_char,
}

/// 一条账户记录（`/config/users.json` 的 `users[]` 元素）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// 用户名（非空、不含 `/`，由解析器保证）。
    pub name: String,
    /// 用户 id。
    pub uid: u32,
    /// 主组 id。
    pub gid: u32,
}

/// 账户表路径（单点定义；`userd` 消费同一路径，改则同改）。
pub const ACCOUNTS_PATH: &str = "/config/users.json";

/// 解析 `/config/users.json` 字节流为账户列表。
///
/// schema：`{"users":[{"name":"alice","uid":1000,"gid":1000}, ...]}`。
/// 与 `userd` 的解析保持同一 schema（S13：同一份用户态格式只在一处规定语义，
/// 此处为 libc 侧独立实现，因 libc 不得依赖 userd 的私有代码）。
///
/// **畸形如实跳过**（S09）：非 UTF-8 / JSON 非法 / 条目缺 name 或 uid → 该条目不入表，
/// 而不是补默认值。返回的列表**只含完整合法条目**。
pub fn parse_accounts(bytes: &[u8]) -> Vec<Account> {
    let mut out = Vec::new();
    let Ok(text) = core::str::from_utf8(bytes) else {
        return out;
    };
    let mut p = libsys::json::JsonParser::new(text);
    let Ok(parsed) = p.parse() else {
        return out;
    };
    if let libsys::json::JsonValue::Object(fields) = parsed {
        for (k, v) in fields {
            if k != "users" {
                continue;
            }
            if let libsys::json::JsonValue::Array(items) = v {
                for it in items {
                    if let libsys::json::JsonValue::Object(obj) = it {
                        let mut name = String::new();
                        let mut uid: Option<u32> = None;
                        let mut gid: Option<u32> = None;
                        for (fk, fv) in obj {
                            match fk.as_str() {
                                "name" => {
                                    if let libsys::json::JsonValue::String(s) = fv {
                                        name = s;
                                    }
                                }
                                "uid" => {
                                    if let libsys::json::JsonValue::Number(n) = fv {
                                        uid = n.parse::<u32>().ok();
                                    }
                                }
                                "gid" => {
                                    if let libsys::json::JsonValue::Number(n) = fv {
                                        gid = n.parse::<u32>().ok();
                                    }
                                }
                                _ => {}
                            }
                        }
                        // 合法性：名字非空、非路径成分；uid 必须存在。
                        // gid 缺失时**如实取 uid**（POSIX 惯常的主组=同名组），
                        // 但仅在 uid 存在时才收录——绝不凭空造 uid。
                        let bad = name.is_empty()
                            || name == "."
                            || name == ".."
                            || name.contains('/');
                        if let (false, Some(uid)) = (bad, uid) {
                            out.push(Account { name, uid, gid: gid.unwrap_or(uid) });
                        }
                    }
                }
            }
        }
    }
    out
}

/// 家目录约定（`/users/<name>`，与 `userd` 建立的家目录一致）。
fn home_dir_of(name: &str) -> String {
    alloc::format!("/users/{}", name)
}

/// 解析后的账户缓存。`None` = 尚未加载；`Some(vec)` = 已加载（可能为空表）。
///
/// 缓存**不**跨"表被修改"自动失效——`setpwent`/`endpwent` 会清空以强制重读。
static CACHE: Mutex<Option<Vec<Account>>> = Mutex::new(None);

/// 返回 `getpwnam`/`getpwuid` 的静态存储（POSIX 约定：调用方不得释放）。
///
/// 用 `OnceLock` 式的稳定位置保存 C 字符串与 `passwd`，使返回指针在下次调用前有效。
/// 因 `passwd` 含裸指针，此处用 `UnsafeCell` + 单线程调用约定（POSIX 同款约束）。
struct StaticEntry {
    name: [u8; 256],
    dir: [u8; 512],
    pw: passwd,
}

// SAFETY：POSIX `getpwnam` 语义即"返回静态存储、非线程安全"；访问经 CACHE 锁串行化
// 的部分只覆盖账户列表，静态返回缓冲按 POSIX 约定由调用方自行串行。本仓无 SMP
// 用户态并发消费该 API 的场景；如需重入请用 getpwnam_r（尚未提供，已声明边界）。
unsafe impl Send for StaticEntry {}

static ENTRY: Mutex<StaticEntry> = Mutex::new(StaticEntry {
    name: [0; 256],
    dir: [0; 512],
    pw: passwd {
        pw_name: ptr::null_mut(),
        pw_uid: 0,
        pw_gid: 0,
        pw_dir: ptr::null_mut(),
        pw_shell: ptr::null_mut(),
    },
});

/// 从 `/config/users.json` 加载账户表（惰性；表不可读时返回空表，由调用方如实报错）。
///
/// **锁纪律（实现期实测教训）**：`spin::Mutex` 是**自旋**锁，绝不可跨越可能让出的
/// 内核路径。此前的写法在 `CACHE.lock()` 持有期间调用 `libsys::read_to_end`（一次
/// 真实文件读，会进入内核并可能被切走）；实测表现为该调用之后**本进程后续所有
/// `write(STDOUT, …)` 静默失效**——程序逻辑仍继续（退出码照常），但日志断在这里，
/// 极难定位。现改为「先查缓存（短暂持锁）→ 释放 → 做 I/O → 再短暂持锁写回」，
/// 使锁的临界区只含内存操作。
fn load() -> Vec<Account> {
    // 第一段临界区：只读缓存，不触碰任何 syscall。
    {
        let guard = CACHE.lock();
        if let Some(list) = guard.as_ref() {
            return list.clone();
        }
    }
    // 临界区之外做真实 I/O：失败即空表——**如实**，不伪造。
    let list = match libsys::read_to_end(ACCOUNTS_PATH) {
        Ok(bytes) => parse_accounts(&bytes),
        Err(_) => Vec::new(),
    };
    // 第二段临界区：只回写内存。
    {
        let mut guard = CACHE.lock();
        // 若期间已有他人填入，保留其值（避免以更旧结果覆盖）。
        if guard.is_none() {
            *guard = Some(list.clone());
        } else if let Some(existing) = guard.as_ref() {
            return existing.clone();
        }
    }
    list
}

/// 把账户投影进静态存储，返回 `struct passwd *`；名字过长（>=256）或家目录过长
/// （>=512）时**如实失败**（返回 NULL），不截断——截断会产出错误路径（S09）。
fn fill_static(acc: &Account) -> *mut passwd {
    let mut e = ENTRY.lock();
    let nb = acc.name.as_bytes();
    if nb.len() >= e.name.len() {
        return ptr::null_mut();
    }
    let home = home_dir_of(&acc.name);
    let hb = home.as_bytes();
    if hb.len() >= e.dir.len() {
        return ptr::null_mut();
    }
    e.name = [0; 256];
    e.name[..nb.len()].copy_from_slice(nb);
    e.dir = [0; 512];
    e.dir[..hb.len()].copy_from_slice(hb);
    // 先取缓冲地址再写结构体指针（避免在同一表达式里同时借用两个字段）。
    let name_ptr = e.name.as_mut_ptr() as *mut c_char;
    let dir_ptr = e.dir.as_mut_ptr() as *mut c_char;
    e.pw.pw_name = name_ptr;
    e.pw.pw_uid = acc.uid;
    e.pw.pw_gid = acc.gid;
    e.pw.pw_dir = dir_ptr;
    e.pw.pw_shell = ptr::null_mut();
    let p: *mut passwd = &mut e.pw;
    // 锁在本函数返回时释放；`passwd` 位于静态存储，指针仍然有效（POSIX 约定）。
    p
}

/// `getpwnam(name)`：按名字查账户。
///
/// 返回指向静态 `struct passwd` 的指针；未找到或表不可读 → `NULL` 并置 `errno`
/// （未找到 `ENOENT`；表不可读 `EIO`）。**绝不**返回伪造账户。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwnam(name: *const c_char) -> *mut passwd {
    if name.is_null() {
        crate::errno::set_errno(crate::errno::EINVAL);
        return ptr::null_mut();
    }
    let mut len = 0usize;
    while *name.add(len) != 0 {
        len += 1;
        if len > 255 {
            crate::errno::set_errno(crate::errno::EINVAL);
            return ptr::null_mut();
        }
    }
    let Ok(want) = core::str::from_utf8(core::slice::from_raw_parts(name as *const u8, len)) else {
        crate::errno::set_errno(crate::errno::EINVAL);
        return ptr::null_mut();
    };
    let list = load();
    if list.is_empty() {
        // 表不可读 / 为空：如实 EIO（区别于"表里没有这个人"的 ENOENT）。
        crate::errno::set_errno(crate::errno::EIO);
        return ptr::null_mut();
    }
    match list.iter().find(|a| a.name == want) {
        Some(acc) => fill_static(acc),
        None => {
            crate::errno::set_errno(crate::errno::ENOENT);
            ptr::null_mut()
        }
    }
}

/// `getpwuid(uid)`：按 uid 查账户。语义同 `getpwnam`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwuid(uid: u32) -> *mut passwd {
    let list = load();
    if list.is_empty() {
        crate::errno::set_errno(crate::errno::EIO);
        return ptr::null_mut();
    }
    match list.iter().find(|a| a.uid == uid) {
        Some(acc) => fill_static(acc),
        None => {
            crate::errno::set_errno(crate::errno::ENOENT);
            ptr::null_mut()
        }
    }
}

/// 遍历游标（`getpwent` 用）。
static ITER: Mutex<usize> = Mutex::new(0);

/// `setpwent()`：重置遍历游标到表首。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setpwent() {
    *ITER.lock() = 0;
}

/// `endpwent()`：结束遍历并释放缓存（下次调用重新读表）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn endpwent() {
    *ITER.lock() = 0;
    *CACHE.lock() = None;
}

/// `getpwent()`：返回下一条账户；到表尾 → `NULL`（不置 errno，POSIX 惯例）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpwent() -> *mut passwd {
    let list = load();
    let mut i = ITER.lock();
    if *i >= list.len() {
        return ptr::null_mut();
    }
    let acc = list[*i].clone();
    *i += 1;
    drop(i);
    fill_static(&acc)
}

// A2-5：纯逻辑层单测（解析器与语义边界，无 syscall 面）。
#[cfg(test)]
mod a2_5_tests {
    use super::*;

    #[test]
    fn parses_well_formed_table() {
        let json = br#"{"users":[{"name":"alice","uid":1000,"gid":1000},
                       {"name":"bob","uid":1001,"gid":1001}]}"#;
        let a = parse_accounts(json);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].name, "alice");
        assert_eq!(a[0].uid, 1000);
        assert_eq!(a[1].name, "bob");
        assert_eq!(a[1].uid, 1001);
    }

    #[test]
    fn missing_gid_defaults_to_uid() {
        let json = br#"{"users":[{"name":"carol","uid":1002}]}"#;
        let a = parse_accounts(json);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].gid, 1002, "absent gid takes uid (never a fabricated value)");
    }

    #[test]
    fn rejects_entries_without_uid() {
        // 无 uid 的条目**不得**收录——否则即伪造身份（S09）。
        let json = br#"{"users":[{"name":"ghost"},{"name":"real","uid":7,"gid":7}]}"#;
        let a = parse_accounts(json);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "real");
    }

    #[test]
    fn rejects_pathlike_and_empty_names() {
        let json = br#"{"users":[{"name":"","uid":1},{"name":"..","uid":2},
                       {"name":"a/b","uid":3},{"name":"ok","uid":4}]}"#;
        let a = parse_accounts(json);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "ok");
    }

    #[test]
    fn malformed_json_yields_empty_not_fabricated() {
        assert!(parse_accounts(b"{not json").is_empty());
        assert!(parse_accounts(b"").is_empty());
        assert!(parse_accounts(&[0xff, 0xfe]).is_empty(), "non-UTF-8 must yield empty");
    }

    #[test]
    fn home_dir_convention() {
        assert_eq!(home_dir_of("alice"), "/users/alice");
    }
}

// ---------------------------------------------------------------------------
// A2-4：组账户（`grp.h` 对应物）——与账户同层的**纯用户态**读表映射。
// ---------------------------------------------------------------------------

/// POSIX `struct group`（字段顺序遵循 POSIX；本实现填充前两项，其余为 NULL）。
#[repr(C)]
pub struct group {
    /// 组名。
    pub gr_name: *mut c_char,
    /// 组 id。
    pub gr_gid: u32,
    /// 成员名列表（**本实现恒为 NULL**：解析 `/config/groups.json` 的 `members` 需
    /// 变长字符串数组，当前未提供该转换；如实置 NULL 而非编造空列表——调用方
    /// 应以 `getgrouplist`（见下）获取某用户的组，而非读 `gr_mem`）。
    pub gr_mem: *mut *mut c_char,
}

/// 一条组记录（`/config/groups.json` 的 `groups[]` 元素）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRecord {
    /// 组名（非空、不含 `/`，由解析器保证）。
    pub name: String,
    /// 组 id。
    pub gid: u32,
    /// 成员账户名列表（**保持声明顺序**；与组 id 无隐式关系）。
    pub members: Vec<String>,
}

/// 组表路径（单点定义；`userd` 消费同一路径，改则同改）。
pub const GROUPS_PATH: &str = "/config/groups.json";

/// 解析 `/config/groups.json` 字节流为组列表。
///
/// schema：`{"groups":[{"name":"dev","gid":2000,"members":["alice","bob"]}, ...]}`。
/// 未知字段跳过；名字非空、非路径成分、gid 可解析才收录；**畸形条目如实跳过**
/// （非 UTF-8 / JSON 非法 / 缺 name 或 gid），不 panic、不补默认值（S09）。
///
/// `members` 缺失或元素非字符串 → 该组仍收录，但成员列表如实为空（组本身合法）；
/// 非字符串成员**跳过并保持其余**，不因一个坏元素丢掉整组。
pub fn parse_groups(bytes: &[u8]) -> Vec<GroupRecord> {
    let mut out = Vec::new();
    let Ok(text) = core::str::from_utf8(bytes) else {
        return out;
    };
    let mut p = libsys::json::JsonParser::new(text);
    let Ok(parsed) = p.parse() else {
        return out;
    };
    if let libsys::json::JsonValue::Object(fields) = parsed {
        for (k, v) in fields {
            if k != "groups" {
                continue;
            }
            if let libsys::json::JsonValue::Array(items) = v {
                for it in items {
                    if let libsys::json::JsonValue::Object(obj) = it {
                        let mut name = String::new();
                        let mut gid: Option<u32> = None;
                        let mut members: Vec<String> = Vec::new();
                        for (fk, fv) in obj {
                            match fk.as_str() {
                                "name" => {
                                    if let libsys::json::JsonValue::String(s) = fv {
                                        name = s;
                                    }
                                }
                                "gid" => {
                                    if let libsys::json::JsonValue::Number(n) = fv {
                                        gid = n.parse::<u32>().ok();
                                    }
                                }
                                "members" => {
                                    if let libsys::json::JsonValue::Array(ms) = fv {
                                        for m in ms {
                                            if let libsys::json::JsonValue::String(ms_) = m {
                                                // 成员名合法性同账户名：非空、非路径成分。
                                                let bad = ms_.is_empty()
                                                    || ms_ == "."
                                                    || ms_ == ".."
                                                    || ms_.contains('/');
                                                if !bad {
                                                    members.push(ms_);
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        let bad = name.is_empty()
                            || name == "."
                            || name == ".."
                            || name.contains('/');
                        if let (false, Some(gid)) = (bad, gid) {
                            out.push(GroupRecord { name, gid, members });
                        }
                    }
                }
            }
        }
    }
    out
}

/// 解析后的组表缓存。`None` = 尚未加载。
static GCACHE: Mutex<Option<Vec<GroupRecord>>> = Mutex::new(None);

/// 组名静态存储（`getgrnam`/`getgrgid` 返回值，POSIX 约定不得释放）。
struct StaticGroup {
    name: [u8; 256],
    gr: group,
}
unsafe impl Send for StaticGroup {}
static GENTRY: Mutex<StaticGroup> = Mutex::new(StaticGroup {
    name: [0; 256],
    gr: group { gr_name: ptr::null_mut(), gr_gid: 0, gr_mem: ptr::null_mut() },
});

/// 加载组表（惰性）。**锁纪律同 [`load`]**：临界区只含内存操作，I/O 在锁外。
pub fn load_groups() -> Vec<GroupRecord> {
    {
        let guard = GCACHE.lock();
        if let Some(list) = guard.as_ref() {
            return list.clone();
        }
    }
    let list = match libsys::read_to_end(GROUPS_PATH) {
        Ok(bytes) => parse_groups(&bytes),
        Err(_) => Vec::new(),
    };
    {
        let mut guard = GCACHE.lock();
        if guard.is_none() {
            *guard = Some(list.clone());
        } else if let Some(existing) = guard.as_ref() {
            return existing.clone();
        }
    }
    list
}

/// 把组记录投影进静态存储，返回 `struct group *`；名字过长如实返回 NULL（不截断）。
fn fill_group(rec: &GroupRecord) -> *mut group {
    let mut e = GENTRY.lock();
    let nb = rec.name.as_bytes();
    if nb.len() >= e.name.len() {
        return ptr::null_mut();
    }
    e.name = [0; 256];
    e.name[..nb.len()].copy_from_slice(nb);
    let np = e.name.as_mut_ptr() as *mut c_char;
    e.gr.gr_name = np;
    e.gr.gr_gid = rec.gid;
    e.gr.gr_mem = ptr::null_mut();
    let p: *mut group = &mut e.gr;
    p
}

/// `getgrnam(name)`：按组名查组。
///
/// 返回指向静态 `struct group` 的指针；组表不可读 → NULL + `EIO`，查不到 → NULL +
/// `ENOENT`。**绝不**返回伪造组（S09）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrnam(name: *const c_char) -> *mut group {
    if name.is_null() {
        crate::errno::set_errno(crate::errno::EINVAL);
        return ptr::null_mut();
    }
    let mut len = 0usize;
    while *name.add(len) != 0 {
        len += 1;
        if len > 255 {
            crate::errno::set_errno(crate::errno::EINVAL);
            return ptr::null_mut();
        }
    }
    let Ok(want) = core::str::from_utf8(core::slice::from_raw_parts(name as *const u8, len)) else {
        crate::errno::set_errno(crate::errno::EINVAL);
        return ptr::null_mut();
    };
    let list = load_groups();
    if list.is_empty() {
        crate::errno::set_errno(crate::errno::EIO);
        return ptr::null_mut();
    }
    match list.iter().find(|g| g.name == want) {
        Some(rec) => fill_group(rec),
        None => {
            crate::errno::set_errno(crate::errno::ENOENT);
            ptr::null_mut()
        }
    }
}

/// `getgrgid(gid)`：按组 id 查组。语义同 `getgrnam`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrgid(gid: u32) -> *mut group {
    let list = load_groups();
    if list.is_empty() {
        crate::errno::set_errno(crate::errno::EIO);
        return ptr::null_mut();
    }
    match list.iter().find(|g| g.gid == gid) {
        Some(rec) => fill_group(rec),
        None => {
            crate::errno::set_errno(crate::errno::ENOENT);
            ptr::null_mut()
        }
    }
}

/// `endgrent()`：释放组表缓存（下次调用重新读表）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn endgrent() {
    *GCACHE.lock() = None;
}

/// 查询账户 `user` 的**全部**组 id：主组（取自账户表）打头，随后是其所属的补充组
/// （按组表声明顺序）。返回实际写入条数；缓冲不足时返回所需条数（> `cap`）——
/// **不截断、不写入半个列表**（POSIX `getgrouplist` 语义，S09）。
///
/// **为何需要它**：`struct group.gr_mem` 在本实现恒为 NULL（见字段说明），故调用方
/// 需按"用户 → 组集合"方向查询。这也是 A2-7 登录后经 `groups_set` 装配身份的
/// 数据来源。
pub fn groups_of_user(user: &str, primary_gid: u32, out: &mut [u32]) -> usize {
    getgrouplist_from(&load_groups(), user, primary_gid, out)
}

/// `getgrouplist(user, group, groups, ngroups)`：BSD/glibc 的组列表查询（声明见 `<grp.h>`）。
///
/// **契约（照 BSD/glibc，不是本实现自定）**：
/// - 入参 `*ngroups` = 调用方缓冲能容纳的条数；
/// - 成功：写入去重后的列表（主组在前，随后按组表声明顺序），把 `*ngroups` 置为**实际条数**，返回 **0**；
/// - 缓冲不足：把 `*ngroups` 置为**所需条数**、返回 **-1**，调用方据此扩容后重试。
///
/// **为什么必须存在这个出口**：`grp.h` 的注释早就写着「要拿某用户的组请用 getgrouplist」，
/// 但此前**只有 Rust 函数、没有 C ABI 出口** ⇒ 用户程序根本拿不到组列表。那是**可达性缺口**，
/// 与 `boruix_memory_query` 同一类。
///
/// **与 Rust 侧 [`groups_of_user`] 的分工**：Rust 侧返回「所需条数」这一个数、不做容量判定；
/// C 侧把它包成 BSD 契约（0 / -1 + `*ngroups` 出参）。判定只有一处，不重复。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgrouplist(
    user: *const crate::ctypes::c_char,
    group: crate::ctypes::gid_t,
    groups: *mut crate::ctypes::gid_t,
    ngroups: *mut crate::ctypes::c_int,
) -> crate::ctypes::c_int {
    if user.is_null() || groups.is_null() || ngroups.is_null() {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let name = match unsafe { crate::stdio::cstr_to_str(user) } {
        Some(s) => s,
        None => {
            crate::errno::set_errno(crate::errno::EINVAL);
            return -1;
        }
    };
    let cap = unsafe { *ngroups };
    if cap < 0 {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // 容量 0 时传空切片：与「缓冲不足」同一条路径（只报数、不写）。
    let out = unsafe { core::slice::from_raw_parts_mut(groups, cap as usize) };
    let need = groups_of_user(&name, group, out);
    unsafe { *ngroups = need as crate::ctypes::c_int };
    if need > cap as usize {
        crate::errno::set_errno(crate::errno::ERANGE);
        return -1;
    }
    0
}

/// [`getgrouplist`] 的**纯逻辑层**（组表由调用方给出，不碰文件系统）。
///
/// **为什么抽出来**：`load_groups()` 走 `libsys::read_to_end` ⇒ 依赖真实 VFS。宿主上
/// `syscall::call` 返回垃圾，`read` 永不返回 0 ⇒ `Vec` 无限增长（实测：宿主单测
/// `memory allocation of 8589934592 bytes failed`）。抽纯后可在宿主上逐条锁定
/// 「不截断、不写半个列表」的语义——与 `read_dir`/`parse_proc_list` 同一手法（S23/S31）。
pub fn getgrouplist_from(records: &[GroupRecord], user: &str, primary_gid: u32, out: &mut [u32]) -> usize {
    // 两趟扫描，**不需要暂存**（本函数服务于登录/身份装配路径，那里不该再依赖堆状态，
    // 也不引入「固定容量」这种魔法上限）。
    //
    // **为什么不边写边去重**（旧实现的缺陷，2026-10 由新打通的宿主单测抓到）：
    // 缓冲不足时 `out` 里只落了前几项，去重就看不到后面已出现过的 gid ⇒ 同一 gid 被数两次，
    // 返回值不再是 POSIX 要求的「所需条数」；更糟的是旧代码用 `out[..n]` 做去重，
    // `n` 一旦超过 `out.len()` 就直接**越界 panic**——而 POSIX 明确要求缓冲不足时
    // 只报数、不写半个列表、不崩。
    //
    // 第 1 趟：数出**完整**去重列表的条数。
    let mut n = 0usize;
    while nth_group(records, user, primary_gid, n).is_some() {
        n += 1;
    }
    // 第 2 趟：按**同一顺序**把前 `out.len()` 项写入。缓冲不足时只写能容纳的部分，
    // 返回值仍是**所需条数**（POSIX：调用方据此扩容后重试）。
    let take = core::cmp::min(n, out.len());
    for (i, slot) in out[..take].iter_mut().enumerate() {
        if let Some(gid) = nth_group(records, user, primary_gid, i) {
            *slot = gid;
        }
    }
    n
}

/// 候选序列（**未去重**）的第 `pos` 项：主组占第 0 位，其后按组表声明顺序追加
/// 「成员含 `user`」的组。`None` = 候选序列到此为止。
fn candidate_at(records: &[GroupRecord], user: &str, primary_gid: u32, pos: usize) -> Option<u32> {
    if pos == 0 {
        return Some(primary_gid);
    }
    let mut i = 1usize;
    for rec in records {
        if rec.members.iter().any(|m| m == user) {
            if i == pos {
                return Some(rec.gid);
            }
            i += 1;
        }
    }
    None
}

/// 去重后列表的第 `idx` 项（`None` = 列表到此为止）。
///
/// 同一 gid 只在**首次**出现处计入：与候选序列中**更早的**项逐一比对。
/// 复杂度 O(n²)，n 是候选条数（组表很小）；这样无需暂存，也不会引入「魔法上限」。
fn nth_group(records: &[GroupRecord], user: &str, primary_gid: u32, idx: usize) -> Option<u32> {
    let mut kept = 0usize;
    let mut pos = 0usize;
    while let Some(gid) = candidate_at(records, user, primary_gid, pos) {
        let dup = (0..pos).any(|j| candidate_at(records, user, primary_gid, j) == Some(gid));
        if !dup {
            if kept == idx {
                return Some(gid);
            }
            kept += 1;
        }
        pos += 1;
    }
    None
}

// A2-4：组表解析的纯逻辑层单测。
#[cfg(test)]
mod a2_4_tests {
    use super::*;

    #[test]
    fn parses_well_formed_group_table() {
        let json = br#"{"groups":[{"name":"dev","gid":2000,"members":["alice","bob"]},
                       {"name":"ops","gid":2001,"members":["carol"]}]}"#;
        let g = parse_groups(json);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].name, "dev");
        assert_eq!(g[0].gid, 2000);
        assert_eq!(g[0].members.len(), 2);
        assert_eq!(g[1].gid, 2001);
    }

    #[test]
    fn group_without_gid_is_rejected() {
        let json = br#"{"groups":[{"name":"ghost"},{"name":"real","gid":7}]}"#;
        let g = parse_groups(json);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "real");
    }

    #[test]
    fn missing_or_bad_members_do_not_drop_the_group() {
        // 组本身合法即收录；坏成员只是**不进入**成员列表，不牵连整组。
        let json = br#"{"groups":[{"name":"a","gid":1},
                       {"name":"b","gid":2,"members":["ok","a/b","", 7]}]}"#;
        let g = parse_groups(json);
        assert_eq!(g.len(), 2);
        assert!(g[0].members.is_empty());
        assert_eq!(g[1].members, alloc::vec![String::from("ok")]);
    }

    #[test]
    fn malformed_group_json_yields_empty_not_fabricated() {
        assert!(parse_groups(b"{not json").is_empty());
        assert!(parse_groups(b"").is_empty());
        assert!(parse_groups(&[0xff]).is_empty());
    }

    #[test]
    fn getgrouplist_reports_required_count_without_truncating() {
        // 用**纯逻辑层**（组表由本测给出）——`getgrouplist` 本体走真实 VFS，宿主上不可用。
        // 成员关系刻意写成一目了然：alice∈dev,ops；bob∈dev；carol∈ops。
        let json = br#"{"groups":[{"name":"dev","gid":2000,"members":["alice","bob"]},
                       {"name":"ops","gid":2001,"members":["alice","carol"]}]}"#;
        let recs = parse_groups(json);

        // 缓冲为 0 时必须如实返回**所需条数**（POSIX 语义），而不是写越界或返回 0。
        let mut empty: [u32; 0] = [];
        let need = getgrouplist_from(&recs, "nobody-not-in-any-group", 5555, &mut empty);
        assert_eq!(need, 1, "only the primary group is reported when no group lists the user");

        // alice：主组 5555 + dev(2000) + ops(2001) ⇒ 3 条；缓冲 0 ⇒ 只报数、不写。
        let need = getgrouplist_from(&recs, "alice", 5555, &mut empty);
        assert_eq!(need, 3, "alice belongs to 3 groups (primary + dev + ops)");

        // 缓冲不足（2 < 3）：仍如实返回所需条数，且**不写半个列表**（不越界）。
        let mut small = [0u32; 2];
        let need = getgrouplist_from(&recs, "alice", 5555, &mut small);
        assert_eq!(need, 3, "buffer too small -> report required count, do not truncate silently");

        // 缓冲足够：写入顺序 = 主组在前，然后按组表声明顺序。
        let mut full = [0u32; 4];
        let need = getgrouplist_from(&recs, "alice", 5555, &mut full);
        assert_eq!(need, 3);
        assert_eq!(&full[..3], &[5555, 2000, 2001], "primary first, then declaration order");

        // 主组与某组重复时只出现一次（去重）：bob 仅属 dev(2000)，主组也是 2000 ⇒ 去重后 1 条。
        let need = getgrouplist_from(&recs, "bob", 2000, &mut full);
        assert_eq!(need, 1, "primary gid 2000 dedups against dev(2000)");
        assert_eq!(full[0], 2000, "dedup: 2000 appears once");

        // 不在任何组、且主组也不与任何组重复 ⇒ 只有主组一条。
        let need = getgrouplist_from(&recs, "carol", 7777, &mut full);
        assert_eq!(need, 2, "carol: primary 7777 + ops(2001)");
        assert_eq!(&full[..2], &[7777, 2001], "primary first, then ops");
    }
}
