#!/usr/bin/env python3
"""errno 两侧同步门：`libc/src/errno.rs` 的常量 vs `libc/include/errno.h` 的宏。

## 为什么需要（同一个坑踩了两次，2026-10）

两边的 errno 表是**同一事实的两份镜像**（S13），必须机械对齐：

- `ENOSYS`：我加进了 Rust 侧，**忘了** `include/errno.h` ⇒ 头文件语法门全绿，
  只有真实 C 程序（tcc 编译 `libcc1.c`）才报 `'ENOSYS' undeclared`。
- `EISDIR`：Rust 侧**一直就有**（`errno.rs:104`），C 头文件里**一直没有** ⇒
  同样是 GCC 的 `fixincludes/fixlib.c` 编译时才暴露。

故本门把「Rust 有、C 没有」与「两侧数值不一致」都当**失败**报出来。

**诚实边界**：头文件**多出**的宏只作**提示**（可能是别名，如 `ENOTSUP`/`EOPNOTSUPP`），
不算失败——别名是合法的。

用法:
    python libc/tools/audit_errno_sync.py
退出码: 0 = 两侧一致；1 = 有缺失/不一致；2 = 找不到文件。
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LIBC = os.path.dirname(HERE)
RS = os.path.join(LIBC, "src", "errno.rs")
HDR = os.path.join(LIBC, "include", "errno.h")


def main():
    for p in (RS, HDR):
        if not os.path.isfile(p):
            print("[FAIL] 找不到 " + p)
            return 2
    rs = open(RS, encoding="utf-8", errors="replace").read()
    hd = open(HDR, encoding="utf-8", errors="replace").read()
    rs_map = {}
    for m in re.finditer(r"pub const (E[A-Z0-9_]+)\s*:\s*i32\s*=\s*(\d+)\s*;", rs):
        rs_map[m.group(1)] = int(m.group(2))
    hd_map = {}
    for m in re.finditer(r"^\s*#define\s+(E[A-Z0-9_]+)\s+(\d+)\s*$", hd, re.M):
        hd_map[m.group(1)] = int(m.group(2))
    missing = sorted(n for n in rs_map if n not in hd_map)
    mismatch = sorted(
        (n, rs_map[n], hd_map[n]) for n in rs_map if n in hd_map and rs_map[n] != hd_map[n]
    )
    extra = sorted(n for n in hd_map if n not in rs_map)
    print("[errno-sync] Rust 侧 %d 个常量；C 头 %d 个宏" % (len(rs_map), len(hd_map)))
    if extra:
        print("  （提示）头文件多出 %d 个（可能是别名，不算失败）: %s" % (len(extra), ", ".join(extra[:10])))
    if not missing and not mismatch:
        print("[OK] 两侧一致")
        return 0
    if missing:
        print("[FAIL] Rust 有、C 头**没有**的 errno（%d 个）——C 程序用它就会 undeclared:" % len(missing))
        for n in missing:
            print("    %-10s = %d" % (n, rs_map[n]))
    if mismatch:
        print("[FAIL] 数值不一致（%d 个）:" % len(mismatch))
        for n, a, b in mismatch:
            print("    %-10s Rust=%d  C 头=%d" % (n, a, b))
    return 1


if __name__ == "__main__":
    sys.exit(main())
