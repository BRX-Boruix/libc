#!/usr/bin/env python3
"""审计 libc 头文件是否**自洽**：每个头文件单独 include 一次，能否通过编译。

## 为什么需要（第 59 轮暴露的自身缺陷，不是预猜）

第 57 轮补 kill 声明时写了 `int kill(pid_t pid, int sig);`，但 <signal.h> 当时**不包含任何头**，
于是「单独 include <signal.h>」的翻译单元报 `error: unknown type name 'pid_t'`。这个缺陷：

- **名字审计查不出**（audit_header_coverage.py 只看"名字出现过没有"）；
- 本机也不容易发现——libc 自己的代码恰好都先包含了 <sys/types.h>；
- **被真实的第三方代码（GMP 的交叉构建）撞了出来**。

故本脚本把「自洽性」也变成一条机械检查。

用法:
    python libc/tools/audit_header_selfcontained.py
环境变量:
    BORUIX_CLANG —— clang 可执行文件（默认取 PATH 里的 clang）
退出码: 0 = 全部自洽；1 = 有头文件不自洽。
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
INC = os.path.join(os.path.dirname(HERE), "include")


def headers():
    out = []
    for root, _dirs, files in os.walk(INC):
        for f in files:
            if f.endswith(".h"):
                rel = os.path.relpath(os.path.join(root, f), INC).replace(os.sep, "/")
                out.append(rel)
    return sorted(out)


def main():
    cc = os.environ.get("BORUIX_CLANG") or "clang"
    hs = headers()
    bad = []
    for h in hs:
        with tempfile.TemporaryDirectory() as td:
            src = os.path.join(td, "t.c")
            with open(src, "w", encoding="utf-8") as fh:
                fh.write("#include <%s>\nint main(void) { return 0; }\n" % h)
            p = subprocess.run(
                [cc, "--target=x86_64-unknown-none", "-ffreestanding", "-fsyntax-only",
                 "-I" + INC, src],
                capture_output=True, text=True, errors="replace")
            if p.returncode != 0:
                bad.append((h, [l for l in p.stderr.strip().splitlines() if l.strip()][:3]))
    print("[headers] 共 %d 个头文件；单独 include 编译失败的 %d 个" % (len(hs), len(bad)))
    for h, err in bad:
        print("  FAIL %s" % h)
        for e in err:
            print("       %s" % e)
    if bad:
        print("\n提示：头文件必须自洽——用到某类型就要自己包含它的来源头。")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
