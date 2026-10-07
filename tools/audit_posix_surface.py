#!/usr/bin/env python3
"""反向清单：**常用 POSIX/libc 入口** vs 本 libc 的**实际导出符号**（真值来源：libc.a）。

## 为什么需要（第 57 轮审计的补集）

audit_header_coverage.py 查「**已导出**但未声明」；它查不出「**整项缺失**」（既没实现也没声明）
——getuid/getgid/dup 就是这么漏掉的，直到外部代码（GMP）撞上来才暴露。

## 为什么用 libc.a 而不是解析 Rust 源码（第 60 轮的教训）

第一版解析源码里的 `#[no_mangle]`，结果**假报了一批 MISSING**：
  - `memcpy`/`memmove`/`memset`/`memcmp` 由 **libsys 的 builtins** 提供（libc 刻意不重复导出），
    源码里没有 `#[no_mangle]`，但它们在 libc.a 里**确实存在**；
  - 有些函数带长文档注释，简单的「属性后 N 行」解析会漏。
**用 `llvm-nm` 读 libc.a 的已定义符号才是真值。** 这同时能查出第四类：

    PHANTOM —— 头文件声明了，但库里**根本没有**这个符号（调用即链接失败）

（实测：`readlink` 就是 PHANTOM——`<unistd.h>` 声明了它，而 libc.a 里没有。）

## 诚实边界（S09）

- 清单是**人工维护**的常用 POSIX.1-2008 / 常见扩展入口，不是"完整 POSIX 标准"；
- MISSING 不等于"必须实现"：无忠实语义的项（无进程组、无信号栈等）应**显式登记为不支持**，
  而不是伪造返回值。本脚本只负责"列全"，判定仍逐项做。

用法:
    python libc/tools/audit_posix_surface.py [--missing-only] [--libc <libc.a>]
环境变量:
    BORUIX_SYSROOT —— 提供 <sysroot>/lib/libc.a（默认取它）
    BORUIX_NM       —— llvm-nm 可执行文件（默认 PATH 里的 llvm-nm）
退出码: 0 = 无 MISSING 且无 PHANTOM；1 = 有。
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LIBC = os.path.dirname(HERE)
INC = os.path.join(LIBC, "include")

CHECKLIST = {
    "进程/身份": ["getpid", "getppid", "getuid", "geteuid", "getgid", "getegid",
                  "setuid", "setgid", "getgroups", "fork", "execv", "execve", "execvp",
                  "waitpid", "wait", "_exit", "atexit", "getpgrp", "setpgid",
                  # posix_spawn 家族：GCC 宿主端口的解锁项（本系统没有 exec 替换，posix_spawn 本来就不要求它）
                  "posix_spawn", "posix_spawnp",
                  "setsid", "kill", "raise", "nice", "uname", "getlogin", "getpwnam",
                  "getpwuid", "getgrnam", "getgrgid", "gethostname"],
    "文件/fd": ["open", "close", "read", "write", "pread", "pwrite", "lseek",
                "dup", "dup2", "fcntl", "ioctl", "fsync", "ftruncate", "truncate",
                "stat", "fstat", "lstat", "chmod", "fchmod", "chown", "fchown",
                "link", "symlink", "readlink", "unlink", "rename", "remove", "mkdir",
                "rmdir", "chdir", "getcwd", "opendir", "readdir", "closedir",
                "rewinddir", "seekdir", "telldir", "scandir", "realpath", "access",
                "umask", "mkstemp", "mkdtemp", "tmpfile", "pipe", "isatty", "ttyname",
                "fileno", "fdopen", "fopen", "freopen", "fclose", "fflush"],
    "stdio": ["printf", "fprintf", "sprintf", "snprintf", "vprintf", "vfprintf",
              "vsprintf", "vsnprintf", "scanf", "sscanf", "fscanf", "getc", "putc",
              "fgetc", "fputc", "getchar", "putchar", "fgets", "fputs", "gets", "puts",
              "fread", "fwrite", "fseek", "ftell", "rewind", "feof", "ferror",
              "clearerr", "perror", "setvbuf", "setbuf", "ungetc", "getline"],
    "字符串/内存": ["strlen", "strcmp", "strncmp", "strcpy", "strncpy", "strcat",
                   "strncat", "strchr", "strrchr", "strstr", "strtok", "strspn",
                   "strcspn", "strpbrk", "strdup", "strndup", "strerror", "strcoll",
                   "memcpy", "memmove", "memset", "memcmp", "memchr", "memmem",
                   "bzero", "bcopy", "swab"],
    "转换": ["atoi", "atol", "atoll", "atof", "strtol", "strtoul", "strtoll",
             "strtoull", "strtod", "strtof", "strtold", "abs", "labs", "llabs",
             "div", "ldiv", "qsort", "bsearch", "rand", "srand", "random"],
    "字符分类": ["isalpha", "isdigit", "isalnum", "isupper", "islower", "isspace",
                "isxdigit", "isprint", "ispunct", "iscntrl", "isgraph", "isblank",
                "isascii", "tolower", "toupper"],
    "时间": ["time", "clock", "gettimeofday", "localtime", "gmtime", "mktime",
             "strftime", "strptime", "difftime", "asctime", "ctime", "nanosleep",
             "clock_gettime", "sleep", "usleep", "alarm"],
    "内存分配": ["malloc", "calloc", "realloc", "free", "alloca", "posix_memalign",
                "memalign", "valloc", "brk", "sbrk", "mmap", "munmap", "mprotect"],
    "信号": ["signal", "sigaction", "sigprocmask", "sigemptyset", "sigfillset",
             "sigaddset", "sigdelset", "sigismember", "sigpending", "sigsuspend",
             "sigaltstack", "sigsetjmp", "siglongjmp", "setjmp", "longjmp",
             "killpg", "abort"],
    "环境/退出": ["getenv", "setenv", "unsetenv", "putenv", "clearenv", "exit",
                  "atexit", "on_exit", "system", "popen", "pclose"],
    "locale/其它": ["setlocale", "localeconv", "iconv", "nl_langinfo", "sysconf",
                    "confstr", "pathconf", "getopt", "getopt_long", "basename",
                    "dirname", "fnmatch", "glob", "wordexp", "regexec", "regcomp",
                    "dlopen", "dlsym", "dlclose", "dlerror"],
}


def lib_symbols(libc_a, nm):
    """libc.a 里**已定义**的 C 符号集合。"""
    p = subprocess.run([nm, "--defined-only", libc_a], capture_output=True, text=True,
                       errors="replace")
    if p.returncode != 0:
        print("llvm-nm 失败：%s" % p.stderr.strip()[:300])
        sys.exit(2)
    out = set()
    for line in p.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 3 and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", parts[2]):
            out.add(parts[2])
    return out


def header_text():
    chunks = []
    for root, _dirs, files in os.walk(INC):
        for f in files:
            if f.endswith(".h"):
                chunks.append(open(os.path.join(root, f), encoding="utf-8", errors="replace").read())
    return "\n".join(chunks)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--missing-only", action="store_true")
    ap.add_argument("--libc", default=None, help="libc.a 路径（默认 <BORUIX_SYSROOT>/lib/libc.a）")
    a = ap.parse_args()

    libc_a = a.libc
    if not libc_a:
        sr = os.environ.get("BORUIX_SYSROOT")
        if sr:
            libc_a = os.path.join(sr, "lib", "libc.a")
    if not libc_a or not os.path.isfile(libc_a):
        print("需要 libc.a：传 --libc <path> 或设 BORUIX_SYSROOT（含 lib/libc.a）")
        return 2
    nm = os.environ.get("BORUIX_NM") or "llvm-nm"

    syms = lib_symbols(libc_a, nm)
    text = header_text()
    print("[posix-surface] 真值来源 %s：已定义符号 %d 个" % (libc_a, len(syms)))

    counts = {"OK": 0, "DECL": 0, "MISSING": 0}
    missing = []
    for domain, names in CHECKLIST.items():
        lines = []
        for n in names:
            if n in syms:
                if re.search(r"\b" + re.escape(n) + r"\b", text):
                    counts["OK"] += 1
                    lines.append(("OK", n))
                else:
                    counts["DECL"] += 1
                    lines.append(("DECL", n))
            else:
                counts["MISSING"] += 1
                lines.append(("MISSING", n))
                missing.append((domain, n))
        if not a.missing_only:
            print("[%s]" % domain)
            for tag, n in lines:
                print("  %-8s %s" % (tag, n))

    # 第四类：头文件声明了、库里没有 —— 调用即链接失败。
    declared = set()
    for m in re.finditer(r"^\s*(?:[A-Za-z_][A-Za-z0-9_ \t*]*?)\b([a-z_][A-Za-z0-9_]*)\s*\(",
                         text, re.M):
        declared.add(m.group(1))
    phantom = sorted(n for n in declared if n not in syms and n in {x for v in CHECKLIST.values() for x in v})

    total = sum(len(v) for v in CHECKLIST.values())
    print("[posix-surface] 清单 %d 项：OK=%d  DECL(已实现未声明)=%d  MISSING(整项缺失)=%d  PHANTOM(声明了库里没有)=%d"
          % (total, counts["OK"], counts["DECL"], counts["MISSING"], len(phantom)))
    if missing:
        print("\nMISSING 明细：")
        for domain, n in missing:
            print("  %-14s %s" % (domain, n))
    if phantom:
        print("\nPHANTOM 明细（头文件声明了但 libc.a 里没有，调用会链接失败）：")
        for n in phantom:
            print("  %s" % n)
    if missing or phantom:
        print("\n注意：MISSING 不等于「必须实现」——无忠实语义的项应**显式登记为不支持**，不要伪造返回值。")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
