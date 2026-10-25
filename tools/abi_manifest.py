#!/usr/bin/env python3
"""libc 符号可见性清单入档（3P5-2a）——生成 / 校验 `docs/abi/libc-symbols.md`。

## 为什么要有这一项（3P5-2 裁定的第一步）

`.so` 化一旦发布即成 **ABI 契约**。裁定（owner，2026-10-18）：分两步——先把「对外暴露哪些
符号」钉死并入档（零契约风险），`.so` 实体制押后到 `PT_INTERP` 接线之后。本脚本就是第一步。

## 单一事实源

清单的**唯一事实源是 C 头文件**，不是那份 markdown：表由 `gen_abi_exports.collect_with_headers()`
从 `libc/include/**` 机械提取——与 version script（`.so` 的导出面）**同一份实现**。
两边各算一遍必然漂移，而漂移在 `.so` 发布后就是悄悄破坏 ABI。

## 用法

    python libc/tools/abi_manifest.py            # 校验（默认；离线、秒级）
    python libc/tools/abi_manifest.py --write    # 重新生成

校验**两份**落盘产物，它们都从同一份头文件派生：

1. `docs/abi/libc-symbols.md`——给人看的入档清单；
2. `libc/abi-exports.txt`——给 `ld.lld` 的 version script（`.so` 的实际导出面）。

第 2 份**必须**一起校验：2026-10-25 的体检发现它**静默漂移了 31 个符号**
（`popen`/`ttyname_r`/`fsync`/`link`/`nice`/`alarm`/`sigsuspend`/`sem_*` …）——
没有任何门禁看它，于是「头文件加了、清单也更新了、`.so` 却导不出来」。

退出码：0 = 一致；1 = 漂移（打印第一处差异）；2 = 文件缺失/头文件读不到。
"""
import collections
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
DOC = os.path.join(ROOT, "docs", "abi", "libc-symbols.md")
# version script 的落盘位置（与 gen_abi_exports.py 的默认目标同一个文件）。
EXPORTS = os.path.join(os.path.dirname(HERE), "abi-exports.txt")

# 控制台可能是 GBK：无法编码的字符（如 ⇒）会让脚本在**输出阶段**崩掉，从而把
# 「检查逻辑判定漂移」与「打印失败」混为一谈。显式降级，宁可打出 ?。
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(errors="replace")

sys.path.insert(0, HERE)
import gen_abi_exports as gen  # noqa: E402

# 由 **rtld** 提供、不由 libc.a 定义的符号：在 libc 头文件里声明只是为了让
# `#include <dlfcn.h>` 的源码能编过。它们是**归属**而非缺口，也是反向对账 PHANTOM=4 的全部内容。
# `.so` 化时**不能**进 global 列表——ld.lld 会因未定义而拒绝。
RTLD_OWNED = ("dlclose", "dlerror", "dlopen", "dlsym")


def render():
    pairs = gen.collect_with_headers()
    if not pairs:
        sys.exit("[abi-manifest] 未提取到任何符号——头文件路径或正则失效（拒绝产出空清单）")
    total = len(pairs)
    rtld = sorted(n for n, _h in pairs if n in RTLD_OWNED)
    by_header = collections.defaultdict(list)
    for n, h in pairs:
        by_header[h].append(n)

    out = []
    out.append("# libc 符号可见性清单（3P5-2a）")
    out.append("")
    out.append("> **自动生成，勿手改。** 生成：`python libc/tools/abi_manifest.py --write`；")
    out.append("> 校验：`python libc/tools/abi_manifest.py`（离线、秒级，已登记进 `tools/checks/run_all.py`）。")
    out.append("")
    out.append("## 这份清单是什么")
    out.append("")
    out.append("`.so` 化（3P5-2 第二步）一旦发布即成 **ABI 契约**，故先把「对外暴露哪些符号」")
    out.append("钉死并入档。**单一事实源是 C 头文件，不是这张表**：表由 `gen_abi_exports.py` 从")
    out.append("`libc/include/**` 机械提取，与 `.so` 的 version script **同一份实现**——两边各算一遍")
    out.append("必然漂移，而漂移在发布后就是悄悄破坏 ABI。")
    out.append("")
    out.append("**导出策略**：只有头文件里声明过的符号对外可见；其余（Rust 内部、core/alloc 的实现")
    out.append("细节）保持 local——既不污染命名空间，也让 `--gc-sections` 能安全裁掉未引用代码。")
    out.append("落盘形态是 **version script**（`{ global: …; local: *; };`），不是 `--dynamic-list`：")
    out.append("后者只能**增加**导出，要限制导出面必须靠 `local: *`。")
    out.append("")
    out.append("## 归属：由 rtld 提供的四个符号")
    out.append("")
    out.append("| 符号 | 说明 |")
    out.append("| --- | --- |")
    for n in RTLD_OWNED:
        out.append("| `%s` | 声明在 `dlfcn.h`（让源码能编过），**定义在 rtld**：它们需要装载器状态 |" % n)
    out.append("")
    out.append("这四项正是反向对账里 `PHANTOM=4` 的全部内容。`.so` 化时**不得**进 global 列表。")
    out.append("")
    out.append("## 清单")
    out.append("")
    out.append("总计 **%d** 个符号：libc 自身 **%d** 个 + rtld 提供 **%d** 个。"
               % (total, total - len(rtld), len(rtld)))
    out.append("")
    out.append("| 头文件 | 符号数 | 符号 |")
    out.append("| --- | --- | --- |")
    for h in sorted(by_header):
        names = sorted(by_header[h])
        cells = ", ".join("`%s`" % n for n in names)
        out.append("| `%s` | %d | %s |" % (h, len(names), cells))
    out.append("")
    out.append("## 与反向对账的关系（两类证据缺一不可）")
    out.append("")
    out.append("本清单证明的是「**打算**导出什么」（来自头文件）。`audit_posix_surface.py` 证明的是")
    out.append("「`libc.a` **实际**定义了哪些符号」（读 `llvm-nm` 的真值）。前者不含实现、后者不含意图：")
    out.append("只有两边都对上，「`.so` 的 ABI 面」才算钉住。差异项由那两个门禁各自暴露，不在本表里重复登记。")
    out.append("")
    return "\n".join(out)


def _report_drift(title, path, have, want, hint):
    """打印第一处差异。只打一行上下文——漂移通常只在少数几行，整表打印反而看不清。"""
    a = have.splitlines()
    b = want.splitlines()
    for i in range(max(len(a), len(b))):
        la = a[i] if i < len(a) else "<缺行>"
        lb = b[i] if i < len(b) else "<多行>"
        if la != lb:
            print("[abi-manifest] 漂移：%s（%s 第 %d 行）" % (title, os.path.basename(path), i + 1))
            print("  落盘: %s" % la[:160])
            print("  头文件: %s" % lb[:160])
            break
    print("  ⇒ %s" % hint)


def _read(path, encoding):
    with open(path, "r", encoding=encoding, newline="") as f:
        return f.read()


def main():
    names = gen.collect()
    if not names:
        sys.exit("[abi-manifest] 未提取到任何符号——头文件路径或正则失效（拒绝产出空清单）")
    doc_text = render()
    exports_text = gen.render_version_script(names)
    if "--write" in sys.argv[1:]:
        os.makedirs(os.path.dirname(DOC), exist_ok=True)
        with open(DOC, "w", encoding="utf-8", newline="\n") as f:
            f.write(doc_text)
        with open(EXPORTS, "w", encoding="ascii", newline="\n") as f:
            f.write(exports_text)
        print("[abi-manifest] 已写入 %s（%d 行）与 %s（%d 个符号）"
              % (DOC, doc_text.count("\n"), os.path.basename(EXPORTS), len(names)))
        return 0
    hint = ("改的是头文件就跑 `python libc/tools/abi_manifest.py --write` 并提交这两份产物；"
            "改的是产物就把它们改回去（两份都是生成的，不是手写的）。")
    for path, want, title, enc in (
        (DOC, doc_text, "入档清单与头文件不一致", "utf-8"),
        (EXPORTS, exports_text, "version script 与头文件不一致", "ascii"),
    ):
        if not os.path.isfile(path):
            print("[abi-manifest] 缺少 %s —— 先跑 --write 生成并提交" % path)
            return 2
        have = _read(path, enc)
        if have != want:
            _report_drift(title, path, have, want, hint)
            return 1
    print("[abi-manifest] OK：入档清单与 version script 均与头文件一致（%d 个符号）" % len(names))
    return 0


if __name__ == "__main__":
    sys.exit(main())
