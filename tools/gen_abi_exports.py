#!/usr/bin/env python3
"""从 libc 的 C 头文件**机械提取** ABI 导出符号，产出 ld.lld 的 --dynamic-list 文件。

为什么必须机械提取（3P5-2「符号可见性清单入档」）：
- 手工维护的清单必然与头文件漂移；
- 而 `.so` 一旦发布即成**契约**，漂移等于悄悄破坏 ABI。

只导出**头文件里声明过的**符号：其余（Rust 内部、core/alloc 的实现细节）保持 local，
既不污染命名空间，也让 --gc-sections 能安全裁掉未引用代码。

用法: python gen_abi_exports.py [输出路径]

**必须由脚本自己写文件**，不要用 shell 重定向：PowerShell 5.1 的 `>` 会写成 UTF-16 并带 BOM，
ld.lld 读到首字节就报 `{ expected`。（本轮实测踩过。）
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
INC = os.path.join(os.path.dirname(HERE), "include")

# C 关键字/类型名：出现在正则捕获里但不是函数名。
KEYWORDS = {
    "int", "void", "char", "long", "short", "unsigned", "signed", "const", "struct",
    "return", "if", "while", "for", "sizeof", "static", "extern", "typedef", "union",
    "enum", "float", "double", "size_t", "ssize_t", "FILE", "va_list", "wchar_t",
    "off_t", "pid_t", "mode_t", "time_t", "clock_t", "div_t", "ldiv_t", "lldiv_t",
    "NULL", "EOF", "errno", "ino_t", "dev_t", "uid_t", "gid_t", "nlink_t",
    "blksize_t", "blkcnt_t", "sighandler_t", "sigset_t", "jmp_buf",
    "_Noreturn", "noreturn", "inline", "restrict", "volatile",
}

# 函数声明：行首（可含 extern）的类型串 + 名字 + "("。
DECL = re.compile(r"^\s*(?:extern\s+)?[A-Za-z_][A-Za-z0-9_ \*]*?\b([a-z_][a-z0-9_]*)\s*\(")
# 属性前缀必须先剥掉：`__attribute__((noreturn)) void exit(int);` 以 `__attribute__` 开头，
# 其后紧跟 `(`，DECL 的「类型串 + 名字 + (」形态匹配不到函数名 → **整条声明被漏掉**。
# 实测漏掉了 exit / _Exit / abort / _exit / longjmp / __assert_fail 六个 C ABI 函数，
# 后果是 `.so` 不导出它们，动态链接的程序调用 exit() 会链接失败。
ATTR = re.compile(r"__attribute__\s*\(\(.*?\)\)", re.S)
# 变量声明（如 extern FILE *stdout;）——libc 的 ABI 里确有导出变量。
VAR = re.compile(r"^\s*extern\s+[A-Za-z_][A-Za-z0-9_ \*]*?\b([a-z_][a-z0-9_]*)\s*;")


def collect():
    names = set()
    for root, _dirs, files in os.walk(INC):
        for fn in sorted(files):
            if not fn.endswith(".h"):
                continue
            path = os.path.join(root, fn)
            with open(path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.split("/*")[0]
                    line = ATTR.sub(" ", line)  # 剥掉属性前缀（见 ATTR 处说明）
                    for rx in (DECL, VAR):
                        m = rx.match(line)
                        if m:
                            n = m.group(1)
                            if n not in KEYWORDS:
                                names.add(n)
    return sorted(names)


def main():
    names = collect()
    if not names:
        sys.exit("未提取到任何符号——头文件路径或正则失效（拒绝产出空清单）")
    # 输出 **version script**（不是 --dynamic-list）：
    # -shared 下所有 global 符号默认都会导出，--dynamic-list 只能**增加**；
    # 要**限制**导出面必须用 version script 的 `local: *;`。
    out = ["{", "  global:"]
    for n in names:
        out.append("    %s;" % n)
    out.append("  local:")
    out.append("    *;")
    out.append("};")
    text = "\n".join(out) + "\n"
    # 默认写到 libc/abi-exports.txt；显式 ASCII + LF，**不经 shell 重定向**（见文件头说明）。
    dest = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(HERE), "abi-exports.txt")
    with open(dest, "w", encoding="ascii", newline="\n") as f:
        f.write(text)
    print("[gen_abi_exports] %d 个导出符号 -> %s" % (len(names), dest))


if __name__ == "__main__":
    main()