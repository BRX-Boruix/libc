#!/usr/bin/env python3
"""审计：libc 的 C 可见导出里，**哪些还没有被任何测试真正碰过**。

## 为什么需要

"审完全部 362 个导出"这句话如果不落到**可数的清单**上，就只是一句声称。本脚本把
"被哪个测试组覆盖"变成机械判据，从而让**缺口可见**（而不是靠"我觉得都看过了"）。

判据是**保守的下界**（名字出现即算覆盖）：
  - `libc/src/conformance.rs@BT@：按标准条款组织的宿主符合性测试组；
  - @BT@shell/src/libc_check.rs@BT@：真机 libccheck 断言组；
  - 定义文件自身的 @BT@#[cfg(test)]@BT@ 模块（模块内局部不变量测试）；
  - @BT@TRIAGED@BT@ 表：由**其他手段**覆盖或**刻意不测**的，每条必须写明理由。

名字没出现在以上任何一处 ⇒ 报 UNCOVERED。注意：名字出现只是**下界**——测试可能只是
顺带提到它；反过来，没出现**也可能**被间接覆盖（例如某测试调了调用它的函数）。
故本脚本**不证明**覆盖率，它只把"从没被任何测试提到过"的那一批挑出来，供人工判定。

用法: python libc/tools/audit_conformance_coverage.py [--verbose]
退出码: 0 = 无未判定项；1 = 有
"""
import argparse
import os
import re
import sys

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(errors="replace")

HERE = os.path.dirname(os.path.abspath(__file__))
LIBC = os.path.dirname(HERE)
ROOT = os.path.dirname(LIBC)
SRC = os.path.join(LIBC, "src")

NO_MANGLE = re.compile(r"^\s*#\[(unsafe\()?no_mangle\)?\]")
FN = re.compile(r'^\s*pub\s+(unsafe\s+)?extern\s+"C"\s+fn\s+([A-Za-z_][A-Za-z0-9_]*)')

# 已判定：由其他手段覆盖，或刻意不测。每条必须写清「谁覆盖」或「为什么不测」。
MATH = "math_verify（宿主对照验证器，27 函数逐点比 ULP）"
TRIAGED = {}
for _m in ("acos asin atan atan2 cbrt ceil copysign cos cosh exp expm1 fabs floor fmod "
           "frexpf hypot ldexpf log log10 log1p log2 modf modff nearbyint nextafter pow "
           "round sin sinh sqrt tan tanh trunc copysignf atan2f hypotf fmodf nextafterf powf").split():
    TRIAGED[_m] = MATH
TRIAGED.update({
    "libc_initialize": "内部初始化入口（由用户程序按需调用，不是 C 标准函数）",
    "boruix_so_abi_version": "BORUIX 私有 ABI 探针",
    "boruix_malloc_corrupt": "BORUIX 私有堆诊断探针",
    "boruix_heap_diag": "BORUIX 私有堆诊断探针",
    "boruix_stdio_write_calls": "BORUIX 私有 stdio 遥测",
    "boruix_stdio_write_bytes": "BORUIX 私有 stdio 遥测",
    "boruix_brk": "malloc 的 brk 封装（BORUIX 私有），由 malloc 自身间接覆盖",
    "boruix_memory_query": "BORUIX 私有内存查询（libsys 直通）",
    "boruix_identity_query": "BORUIX 私有身份查询（libccheck 经它取 uid）",
    "boruix_identity_set": "BORUIX 私有身份设置（libsys 直通）",
    "__boruix_init_environ": "libsys 的 environ 引导，由进程启动路径覆盖",
    "__errno_location": "errno 机制本身（所有 errno 断言都在用它）",
    "__assert_fail": "断言失败路径（触发即中止进程，无法在通过态里断言）",
    "__sigsetjmp_prepare": "setjmp 内部辅助（与 header_coverage 同一判定）",
})

# ---- 需要内核/终止路径：由真机覆盖（未逐函数断言）或无法在通过态断言 ----
KERNEL_ONLY = (
    "需要内核（VFS / syscall / 堆）：宿主构造不出，只在真机可验。真机 libccheck 现有 271 条断言在**整体上**覆盖了这些子系统（文件流、进程、信号、信号量、内存映射、spawn），但**未逐函数**断言——本表把它们登记为「已知的真机专属面」，逐函数扩展列在 docs 的下一轮入口里。"
)
TERMINATING = "进程终止/长跳转路径：调用即结束进程或跳走，无法在**通过态**里断言；语义由真机的启动/退出路径覆盖。"
EXTRA = {
    "_getopt_internal": "getopt/getopt_long 的内部实现（S15 单点），由这两个公开入口覆盖",
    "gets": "已废弃且不安全；按 POSIX 从 stdin 读（真机专属）",
}
for _n in """
access aligned_alloc calloc chdir chown clearerr clock_gettime dup fdopen feof ferror fflush fgetpos fgets fileno fork fprintf fputc fputs freopen fseek fsetpos ftell ftruncate getc getchar getcwd getdelim getegid geteuid getppid gettid gettimeofday globfree lseek lstat mbsrtowcs memalign mkdtemp mkstemp mktemp mprotect munmap nanosleep perror posix_spawn posix_spawn_file_actions_addclose posix_spawn_file_actions_adddup2 posix_spawn_file_actions_addopen posix_spawn_file_actions_destroy posix_spawn_file_actions_init posix_spawnattr_destroy posix_spawnattr_getflags posix_spawnattr_init posix_spawnattr_setflags posix_spawnp pread putc putchar puts pwrite readlink realpath rewind rewinddir sbrk scandir scanf seekdir sem_destroy sem_getvalue sem_init sem_post sem_trywait sem_wait setbuf setgid setuid setvbuf sscanf strdup strndup symlink telldir tmpfile truncate umask usleep valloc vfprintf vprintf vsnprintf vsprintf waitpid wcsrtombs yield_sys
""".split():
    TRIAGED[_n] = KERNEL_ONLY
for _n in """
_exit abort atexit exit on_exit siglongjmp
""".split():
    TRIAGED[_n] = TERMINATING
TRIAGED.update(EXTRA)


def exported_functions():
    out = {}
    for name in sorted(os.listdir(SRC)):
        if not name.endswith(".rs"):
            continue
        path = os.path.join(SRC, name)
        lines = open(path, encoding="utf-8").read().splitlines()
        pending = 0
        for i, line in enumerate(lines):
            if NO_MANGLE.match(line):
                pending = 6
                continue
            if pending > 0:
                m = FN.match(line)
                if m:
                    out[m.group(2)] = (name, i + 1)
                    pending = 0
                else:
                    pending -= 1
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--verbose", action="store_true")
    a = ap.parse_args()

    exports = exported_functions()
    conf = open(os.path.join(SRC, "conformance.rs"), encoding="utf-8").read()
    check = open(os.path.join(ROOT, "shell", "src", "libc_check.rs"), encoding="utf-8").read()
    src_cache = {}

    def own_tests(fname, sym):
        if fname not in src_cache:
            src_cache[fname] = open(os.path.join(SRC, fname), encoding="utf-8").read()
        t = src_cache[fname]
        i = t.find("#[cfg(test)]")
        return i >= 0 and re.search(r"\b" + re.escape(sym) + r"\b", t[i:]) is not None

    covered, triaged, uncovered = [], [], []
    for sym, (fname, ln) in sorted(exports.items()):
        if re.search(r"\b" + re.escape(sym) + r"\b", conf):
            covered.append((sym, "conformance"))
        elif re.search(r"\b" + re.escape(sym) + r"\b", check):
            covered.append((sym, "libccheck"))
        elif own_tests(fname, sym):
            covered.append((sym, "module-tests"))
        elif sym in TRIAGED:
            triaged.append((sym, TRIAGED[sym]))
        else:
            uncovered.append((sym, fname, ln))

    if a.verbose:
        for sym, how in covered:
            print("  ok   %-32s %s" % (sym, how))
    print(
        "[coverage] C 可见导出 %d 个：测试组直接提到 %d，已判定由其他手段覆盖/刻意不测 %d，"
        "**未判定 %d**" % (len(exports), len(covered), len(triaged), len(uncovered))
    )
    for sym, why in triaged:
        print("  TRIAGED %-30s %s" % (sym, why))
    for sym, fname, ln in uncovered:
        print("  UNCOVERED %-28s (%s:%d)" % (sym, fname, ln))
    if uncovered:
        print("\\n提示：每一个都该被显式判定一次——补测试，或登记进 TRIAGED 并写明理由。")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
