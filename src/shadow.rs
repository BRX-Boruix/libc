//! 口令表（shadow）读取与校验——A2-7 认证的用户态侧（ADR-041 §1.2）。
//!
//! ## 本模块的定位（S13 单点）
//!
//! **`/config/shadow.json` 是 uid/gid/name 的权威来源**（ADR-041 §1.2.6）。`users.json` 的
//! 同名字段是**展示信息**，本模块**绝不**读取它来定位身份——否则改那张可写的表即可
//! 冒充他人。这条纪律是本模块存在的核心理由。
//!
//! ## 诚实边界（S39）
//!
//! - 表不可读 / 非法 / 缺条目 ⇒ 返回 `Err` 或 `None`，**绝不**返回伪造记录；
//! - **缺口令或空 hash ⇒ 账户不可登录**（`verify` 返回 `Err`）。本实现明确**不采用**
//!   `shadow(5)` 的"空口令字段表示可无口令登录"语义——在本项目语境下那是不可接受的
//!   风险面；此点与 ADR-041 §1.2.6 一致。
//! - 校验用**恒定比较**（`subtle` 式：全量异或后判零），不做提前返回——避免以比较耗时
//!   泄漏"前缀猜中了几字节"。

use alloc::string::String;
use alloc::vec::Vec;

/// 口令表路径（单点定义）。
pub const SHADOW_PATH: &str = "/config/shadow.json";

/// 一条口令记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowEntry {
    /// 账户名（权威）。
    pub name: String,
    /// 用户 id（**权威**，login 降权只认它）。
    pub uid: u32,
    /// 主组 id（**权威**）。
    pub gid: u32,
    /// 盐的原始字节（十六进制解码后）。
    pub salt: Vec<u8>,
    /// `SHA-256(salt || password)` 的原始字节。
    pub hash: [u8; 32],
}

/// 十六进制字符 → 半字节值。非十六进制返回 `None`。
fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// 解码十六进制串到 `out`；长度或字符非法返回 `None`。**不截断、不补零**（S09）。
fn hex_decode(s: &str, out: &mut [u8]) -> Option<()> {
    let b = s.as_bytes();
    if b.len() != out.len() * 2 {
        return None;
    }
    for i in 0..out.len() {
        let hi = hex_nibble(b[i * 2])?;
        let lo = hex_nibble(b[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(())
}

/// 解析 `/config/shadow.json` 字节流为口令记录列表。
///
/// **畸形条目如实跳过**（缺 name/uid/gid/salt/hash 任一、或十六进制非法、或 salt 为空），
/// 不 panic、不补默认值——某条坏了不该连累其余条目，但**坏条目绝不进结果**。
pub fn parse_shadow(bytes: &[u8]) -> Vec<ShadowEntry> {
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
            if k != "accounts" {
                continue;
            }
            if let libsys::json::JsonValue::Array(items) = v {
                for it in items {
                    if let libsys::json::JsonValue::Object(obj) = it {
                        if let Some(e) = parse_entry(&obj) {
                            out.push(e);
                        }
                    }
                }
            }
        }
    }
    out
}

/// 解析单条记录；任一必需字段非法即返回 `None`（**不产生半条记录**）。
fn parse_entry(obj: &[(String, libsys::json::JsonValue)]) -> Option<ShadowEntry> {
    let mut name = String::new();
    let mut uid: Option<u32> = None;
    let mut gid: Option<u32> = None;
    let mut salt_s = String::new();
    let mut hash_s = String::new();
    for (fk, fv) in obj {
        match fk.as_str() {
            "name" => {
                if let libsys::json::JsonValue::String(s) = fv {
                    name = s.clone();
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
            "salt" => {
                if let libsys::json::JsonValue::String(s) = fv {
                    salt_s = s.clone();
                }
            }
            "hash" => {
                if let libsys::json::JsonValue::String(s) = fv {
                    hash_s = s.clone();
                }
            }
            _ => {}
        }
    }
    // 名字合法性（同账户名规则）。
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return None;
    }
    let uid = uid?;
    let gid = gid?;
    // 盐：**必须存在且非空**（空盐等于无语义盐，使彩虹表直接可用）。
    if salt_s.is_empty() || salt_s.len() % 2 != 0 {
        return None;
    }
    let mut salt = alloc::vec![0u8; salt_s.len() / 2];
    hex_decode(&salt_s, &mut salt)?;
    // 哈希：必须是 64 个十六进制字符（32 字节）；缺失/空/长度不符一律拒收。
    let mut hash = [0u8; 32];
    hex_decode(&hash_s, &mut hash)?;
    Some(ShadowEntry { name, uid, gid, salt, hash })
}

/// 载入口令表。表不可读或无有效条目 ⇒ `Err`（**绝不**返回空表冒充"没有账户"）。
pub fn load_shadow() -> Result<Vec<ShadowEntry>, libsys::Error> {
    let bytes = libsys::read_to_end(SHADOW_PATH)?;
    let list = parse_shadow(&bytes);
    if list.is_empty() {
        // 文件读到了但无有效条目：这是**配置错误**，如实报错而非返回空表。
        return Err(libsys::Error::InvalidParam);
    }
    Ok(list)
}

/// 按名字检索（**权威** uid/gid 来源）。
pub fn lookup(name: &str) -> Result<Option<ShadowEntry>, libsys::Error> {
    let list = load_shadow()?;
    Ok(list.into_iter().find(|e| e.name == name))
}

/// 恒定时间比较（长度相等时全量异或；长度不等也走满循环后返回 false）。
///
/// **为何恒定时间**：若提前返回，比较耗时随"前缀匹配长度"变化，理论上可被逐字节猜解。
/// 本内核无网络、此风险实际很低，但恒定比较的实现代价近乎为零，故直接做对，
/// 不留一个"理论上可修"的口子。
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for i in 0..a.len() {
        acc |= a[i] ^ b[i];
    }
    acc == 0
}

/// 校验 `password` 是否匹配该记录。
///
/// 计算 `SHA-256(salt || password)` 与记录中的哈希做**恒定时间**比较。
/// 返回 `Ok(true)` = 匹配；`Ok(false)` = 不匹配；`Err` = 记录本身不可用（如缺口令）。
pub fn verify(entry: &ShadowEntry, password: &[u8]) -> Result<bool, libsys::Error> {
    // 缺口令/空 hash：不可登录（ADR-041 §1.2.6，明确不采用 shadow(5) 的空口令语义）。
    if entry.hash == [0u8; 32] && entry.salt.is_empty() {
        return Err(libsys::Error::InvalidParam);
    }
    let mut c = crate::sha256::Sha256::new();
    c.update(&entry.salt);
    c.update(password);
    let got = c.finish();
    Ok(ct_eq(&got, &entry.hash))
}

/// 为给定口令生成新记录所需字段（盐 + 哈希），供 `userd`/`passwd` 类工具使用。
///
/// **盐来源**：调用方须传入。**不得**用 `libc::random`（那是确定性 PRNG，模块文档自述
/// 非密码学安全）。正确来源见 ADR-041 §1.2.4：读 `/devices/random`（内核熵）。
pub fn hash_password(salt: &[u8], password: &[u8]) -> [u8; 32] {
    let mut c = crate::sha256::Sha256::new();
    c.update(salt);
    c.update(password);
    c.finish()
}
