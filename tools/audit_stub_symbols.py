#!/usr/bin/env python3
"""桩符号普查：找出「符号存在、但函数体没做事」的 libc 导出。

## 为什么需要（盲区，不是洁癖）

`libc/tools/audit_posix_surface.py` 用 `llvm-nm` 读 `libc.a`，只能证明**符号存在**。
一个只写 `set_errno(ENOTSUP); -1` 的桩，符号照样在 ⇒ 对账把它算作「已实现」。
2026-10 实测：`fseek`/`ftell` 就是这样一对桩，靠系统内运行时验收（`tools/3psrc/libcc1`）才现形。

## 做法

对 `audit_posix_surface.py` 清单里的每个名字，在 `libc/src/*.rs` 里找到它的定义，取**函数体**
（按花括号定界，遇下一个定义即停；**先剥掉注释**——否则文档里提一句 ENOTSUP 就会误报，
本脚本首版正是这么误报了 `feof`/`ferror`），然后判定：

  - 体内出现 `ENOTSUP` / `NotSupported`  ⇒ **有「如实不支持」分支**（打出来供分诊）
  - 体本身只是「返回 `-1` / `0` / NULL / false」 ⇒ **空实现**

**诚实边界（S09）**：这是**启发式，不是证明**。
  - 「有 ENOTSUP 分支」**大多不是缺陷**——`waitpid(options != 0)`、`lseek(SEEK_CUR 未定位)`、
    `glob(未实现的旗标)` 都是**如实拒绝**，正是正确行为。它的价值是**缩小人工复核范围**。
  - 没报出来的**也可能**是桩（体内有别的语句但实际没做事）。
故本脚本**不替代**人工分诊，更不替代系统内运行时验收。

用法:
    python libc/tools/audit_stub_symbols.py [--src <libc/src>]
退出码: 0 = 未发现可疑项；1 = 有可疑项（需人工分诊）。
"""
import argparse
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LIBC = os.path.dirname(HERE)

# ---------------------------------------------------------------------------
# 已分诊清单：这些名字**预期**会被启发式命中，且已逐项核实为「正确行为」。
#
# 为什么要有它：本脚本的价值是**门禁**——「有没有新出现的可疑项」。若不记录已分诊的，
# 每次都会报同一批，门禁就失效了（人会开始无视它）。每条必须带**具体理由**，
# 不允许只写「已知」（那样等于把糊涂账固化）。
# ---------------------------------------------------------------------------
TRIAGED = {
    "execvp": "B 类：本系统没有替换进程映像的 syscall（只有派生）——详见 docs/TODO/libc-posix-surface.md",
    "fcntl": "部分实现：F_DUPFD 真实支持（含位置表复制）；其余命令如实 ENOTSUP，不伪造",
    "fflush": "刻意的空操作：本 libc 无用户态缓冲，无数据可冲——函数文档已写明，且与 setvbuf 判不支持同一决定",
    "glob": "未实现的旗标（GLOB_BRACE/GLOB_TILDE 等）如实拒绝，绝不静默忽略",
    "lseek": "SEEK_CUR 对从未被定位过的 fd 无法得知当前位置（内核不暴露），如实 ENOTSUP",
    "mmap": "只支持匿名映射；非匿名 / MAP_SHARED / 指定地址如实 ENOTSUP（文件映射未实现）",
    "waitpid": "仅 options==0 与 WNOHANG 支持；WUNTRACED 等如实 ENOTSUP（本系统无作业控制）",
}

FN_RE = re.compile(r"^\s*(?:pub\s+)?(?:unsafe\s+)?(?:extern\s+\"C\"\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(")


def load_checklist():
    """复用反向对账的清单（S15 单点：清单只有一份）。"""
    path = os.path.join(HERE, "audit_posix_surface.py")
    spec = importlib.util.spec_from_file_location("audit_posix_surface", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    names = []
    for _domain, lst in mod.CHECKLIST.items():
        names.extend(lst)
    return sorted(set(names))


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"//[^\n]*", "", text)
    return text


def body_of(lines, i):
    """从定义行 i 起取函数体：遇下一个顶层定义即停；再按花括号深度收口。**先剥注释**。"""
    out = []
    depth = 0
    started = False
    for j in range(i, min(len(lines), i + 120)):
        line = lines[j]
        if j > i and FN_RE.match(line):
            break
        out.append(line)
        depth += line.count("{") - line.count("}")
        if "{" in line:
            started = True
        if started and depth <= 0:
            break
    return strip_comments("\n".join(out))


def scan_source(src_dir):
    defs = {}
    for root, _dirs, files in os.walk(src_dir):
        for f in files:
            if not f.endswith(".rs"):
                continue
            p = os.path.join(root, f)
            lines = open(p, encoding="utf-8", errors="replace").read().splitlines()
            for i, line in enumerate(lines):
                m = FN_RE.match(line)
                if not m:
                    continue
                defs.setdefault(m.group(1), (p, i + 1, body_of(lines, i)))
    return defs


def classify(body):
    if "ENOTSUP" in body or "NotSupported" in body:
        ctx = ""
        for ln in body.splitlines():
            if "ENOTSUP" not in ln and "NotSupported" not in ln:
                continue
            # 排除**字符串表项**（如 strerror 的 `e::ENOTSUP => b"..."`）：那是查表，
            # 不是「不支持」分支。本脚本首版把 strerror 误报过。
            if 'b"' in ln:
                continue
            ctx = ln.strip()
            break
        if not ctx:
            return None
        return ("如实不支持分支", ctx[:72])
    tail = body.split("{", 1)[-1] if "{" in body else body
    stripped = re.sub(r"[\s;]", "", tail)
    for trivial in ("-1}", "0}", "core::ptr::null_mut()}", "false}", "()}"):
        if stripped == trivial:
            return ("空实现", "体只是 `" + trivial[:-1] + "`")
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default=os.path.join(LIBC, "src"))
    a = ap.parse_args()
    if not os.path.isdir(a.src):
        print("[FAIL] 找不到源码目录: " + a.src)
        return 2
    names = load_checklist()
    defs = scan_source(a.src)
    hits = []
    for n in names:
        d = defs.get(n)
        if not d:
            continue
        p, ln, body = d
        c = classify(body)
        if c:
            hits.append((n, p, ln, c[0], c[1]))
    print("[stub-scan] 清单 %d 项；在 %s 找到 %d 个定义" % (len(names), a.src, len(defs)))
    if not hits:
        print("[OK] 未发现可疑项")
        return 0
    new_hits = [h for h in hits if h[0] not in TRIAGED]
    print("  命中 %d 个：已分诊 %d，**新增 %d**" % (len(hits), len(hits) - len(new_hits), len(new_hits)))
    for n, p, ln, kind, why in hits:
        tag = "已分诊" if n in TRIAGED else "★新增"
        print("    %-14s %s:%d  [%s] %s" % (n, os.path.relpath(p, LIBC), ln, kind, why))
        if n in TRIAGED:
            print("        └ 理由：" + TRIAGED[n])
    if not new_hits:
        print("[OK] 无新增可疑项（已分诊项的理由见上；如结论变化请改 TRIAGED）")
        return 0
    print("[!] 有 %d 个**新增**可疑项，需人工分诊：" % len(new_hits))
    return 1


if __name__ == "__main__":
    sys.exit(main())
