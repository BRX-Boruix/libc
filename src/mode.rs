//! `mode` 的约定适配：POSIX 权限位 → 内核的 3 个布尔。
//!
//! **为什么需要适配层**：内核的 `Permissions` 只有 `readable`/`writable`/`executable` 三个布尔
//! （见 `sys/stat.h` 的诚实边界），**不存在** owner/group/other 权限矩阵。而 `open`/`mkdir` 的
//! `mode` 参数按 POSIX 是 `0644`/`0755` 这类**位掩码**。此前 libc 直接取 `mode` 的**最低 3 位**
//! 当 r/w/x（`mode & 0b100` 当 readable）——那与调用方传的 `0644` 语义**不通**：
//! `0644 & 0b100 = 4 ≠ 0` ⇒ readable，而 `0644 & 0b010 = 0` ⇒ **not writable**，
//! 于是一个要求「owner 可读写」的文件被建成**只读**。
//!
//! **同时 `stat` 早就说 POSIX 语**（`st_mode & 0o100` 当 S_IXUSR、`0o120000` 当 S_IFLNK）
//! ⇒ 两套约定此前**互相矛盾**。本模块把 `open`/`mkdir` 这一侧对齐到 POSIX。
//!
//! **折叠方向取 owner 位**，与 `sys/stat.h` 已写明的填法（「owner 为真值、组/其他镜像 owner」）
//! 一致 ⇒ 建文件与读回 `st_mode` 两侧自洽。

use crate::ctypes::mode_t;

/// POSIX 权限位（与 `sys/stat.h` 的 `S_I*` 同一事实）。
pub const S_IRUSR: mode_t = 0o400;
pub const S_IWUSR: mode_t = 0o200;
pub const S_IXUSR: mode_t = 0o100;

/// 把 POSIX `mode` 与 `umask` 折成内核的 `(readable, writable, executable)`。
///
/// 规则：先 `mode & !umask`（POSIX 语义），再取 **owner 位**。
///
/// **诚实边界（S09）**：内核无法表示 owner/group/other 的差异，故 `umask` 里**清除组/其他**的
/// 那些位在本系统上**不可观测**（例如 `umask 022` 对 `0644` 毫无影响——它本来就没有组/其他写位）。
/// 只有**清除 owner 位**（如 `umask 0o200`）才可观测。这是内核权限模型的边界，不是本函数的近似。
pub fn perms_from_mode(mode: mode_t, umask: mode_t) -> (bool, bool, bool) {
    let m = mode & !umask;
    (
        m & S_IRUSR != 0,
        m & S_IWUSR != 0,
        m & S_IXUSR != 0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_read_write_is_readable_and_writable() {
        // POSIX 0644 = owner rw / group r / other r ⇒ 本内核只表达 owner 那三位 ⇒ (true, true, false)
        assert_eq!(perms_from_mode(0o644, 0), (true, true, false));
    }

    #[test]
    fn owner_read_only() {
        assert_eq!(perms_from_mode(0o400, 0), (true, false, false));
    }

    #[test]
    fn owner_executable() {
        assert_eq!(perms_from_mode(0o755, 0), (true, true, true));
    }

    #[test]
    fn umask_clears_owner_write_when_it_covers_the_owner_bit() {
        // umask 0o200 清掉 owner 写 ⇒ 可观测
        assert_eq!(perms_from_mode(0o644, 0o200), (true, false, false));
    }

    #[test]
    fn umask_clearing_group_other_is_unobservable() {
        // 诚实边界：umask 022 对 0644 无影响（本内核不表示组/其他）
        assert_eq!(perms_from_mode(0o644, 0o022), perms_from_mode(0o644, 0));
    }

    #[test]
    fn zero_mode_is_no_permissions() {
        assert_eq!(perms_from_mode(0, 0), (false, false, false));
    }

    /// 回归锁：这是**改动前**的行为——`0644` 被解成**只读**（最低 3 位当 r/w/x）。
    /// 它必须是**错**的，否则本次适配没有意义。
    #[test]
    fn legacy_lowest_three_bits_interpretation_is_wrong() {
        let legacy_readable = (0o644 & 0b100) != 0;
        let legacy_writable = (0o644 & 0b010) != 0;
        assert!(legacy_readable);
        assert!(!legacy_writable, "旧约定把 0644 解成只读——这正是要修的");
        assert_ne!(
            (legacy_readable, legacy_writable, (0o644 & 0b001) != 0),
            perms_from_mode(0o644, 0)
        );
    }
}