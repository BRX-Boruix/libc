#!/usr/bin/env python3
"""审计 libc 的 C 可见面：`#[no_mangle]` 导出的符号里，有哪些**任何头文件都没提到**。

## 为什么需要（3P6-2 的教训，不是预猜）

连续三个由 GCC 驱动暴露的缺口都是**同一类**：

| 符号 | 实现（libc/src/*.rs） | 声明（libc/include/*.h） |
| --- | --- | --- |
| `getpid` | 早有 | **缺**（tcc 报 implicit declaration） |
| `dup2` | 早有（内核+libsys 也有） | **缺**（GCC/libiberty 报 undeclared） |
| `EINTR` | 早有（errno.rs） | **缺**（GCC/simple-object.c 报 undeclared） |

这不是巧合，是「两份事实来源」的结构性问题。本脚本把这类缺口**一次列全**，
而不是等外部编译器一个一个撞出来。

## 判据（保守：宁可漏报，不可误报）

- **导出面**：`libc/src/*.rs` 中 `#[no_mangle]` / `#[unsafe(no_mangle)]` 之后紧跟的
  `pub [unsafe] extern "C" fn NAME`。
- **声明面**：`libc/include/**/*.h` 里**出现过该标识符**（词边界匹配）。出现在注释里也算
  「提到」——所以本脚本只报「**完全没提到**」的符号，这是保守方向。

用法:
    python libc/tools/audit_header_coverage.py [--verbose]
退出码: 0 = 无缺口；1 = 有缺口（便于接进 CI）。
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LIBC = os.path.dirname(HERE)
SRC = os.path.join(LIBC, "src")
INC = os.path.join(LIBC, "include")

NO_MANGLE = re.compile(r"^\s*#\[(unsafe\()?no_mangle\)?\]")
FN = re.compile(r'^\s*pub\s+(unsafe\s+)?extern\s+"C"\s+fn\s+([A-Za-z_][A-Za-z0-9_]*)')


def exported_functions():
    """返回 {symbol: (file, line)}：libc/src 里 C 可见的函数导出。"""
    out = {}
    for name in sorted(os.listdir(SRC)):
        if not name.endswith(".rs"):
            continue
        path = os.path.join(SRC, name)
        lines = open(path, encoding="utf-8").read().splitlines()
        pending = 0
        for i, line in enumerate(lines):
            if NO_MANGLE.match(line):
                pending = 6          # 属性之后最多再找 6 行
                continue
            if pending > 0:
                m = FN.match(line)
                if m:
                    out[m.group(2)] = (name, i + 1)
                    pending = 0
                else:
                    pending -= 1
    return out


def strip_comments(text):
    """去掉 C 注释。

    **为什么必须去注释（第 60 轮的假阴性教训）**：本审计的判据是"名字出现过即算声明"，
    于是**只出现在注释里**的名字会被误判为已声明——实测 `fcntl` 就是这样漏掉的：
    实现在 unistd.rs、<fcntl.h> 里只在别处的注释中提到，而 wave2.c 在系统内编译时
    报 `implicit declaration of function 'fcntl'`。
    """
    out = []
    i = 0
    n = len(text)
    while i < n:
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def header_text():
    chunks = []
    for root, _dirs, files in os.walk(INC):
        for f in files:
            if f.endswith(".h"):
                raw = open(os.path.join(root, f), encoding="utf-8", errors="replace").read()
                chunks.append(strip_comments(raw))
    return "\n".join(chunks)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--verbose", action="store_true", help="连已覆盖的也列出来")
    a = ap.parse_args()

    exports = exported_functions()
    text = header_text()
    missing = []
    for sym, (f, ln) in sorted(exports.items()):
        if re.search(r"\b" + re.escape(sym) + r"\b", text):
            if a.verbose:
                print("  ok   %-32s (%s:%d)" % (sym, f, ln))
        else:
            missing.append((sym, f, ln))

    print("[audit] C 可见导出共 %d 个；**任何头文件都没提到**的 %d 个" % (len(exports), len(missing)))
    for sym, f, ln in missing:
        print("  MISS %-32s (%s:%d)" % (sym, f, ln))
    if missing:
        print("\n提示：这些不全是缺陷——有些是刻意内部（例如只在 libsys 侧使用的入口）。")
        print("      但**每一个都该被显式判定一次**：要么补进头文件，要么在这里登记为内部符号。")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
