# BORUIX libc

BORUIX 的用户态 C 标准库（Rust 实现 + C ABI，构建于 libsys 之上）。

## 架构（ADR-001 语言工具链 + 本 crate 设计）

```text
kernel(syscall)  ←  libsys(薄封装, 提供 _start/user_main)  ←  libc(本 crate, C ABI)  ←  用户程序
```

- **libsys**：syscall 薄封装（\`int 0x80\`，bit63 错误标记），提供进程入口 \`_start\`→\`user_main\`。
- **libc**：\`extern "C"\` 函数集，供 no_std Rust bin crate 经 \`unsafe extern\` 调用，
  也产出 staticlib 供未来原生 C 工具链链接。**不接管进程入口**（\`_start\` 仍由 libsys 提供）。
- crate-type：\`["rlib", "staticlib"]\`，\`#![cfg_attr(not(test), no_std)]\`，
  \`#![feature(c_variadic)]\`（printf 家族可变参数）。

## 已实现模块与函数

| 模块 | 函数 | 真实数据链路（S06） |
|---|---|---|
| `malloc` | malloc / free / realloc / calloc / malloc_usable_size / posix_memalign / aligned_alloc | 经 libsys `brk` 扩展堆，自有空闲链表分配器；posix_memalign 支持任意 2 的幂对齐；加固：canary 越界守卫、free 0xDD 毒化、double-free 检测 |
| \`string\` | memcpy/memmove/memset/memcmp/memchr/strlen/strnlen/strcmp/strncmp/strcpy/strncpy/strcat/strncat/strchr/strrchr/strstr/strdup/strspn/strcspn/strpbrk/strtok/strtok_r | 纯逻辑，可移植（host 可单测）；strtok_r 线程安全（saveptr） |
| \`ctype\` | isalpha/isalnum/isupper/.../tolower/toupper | 纯逻辑 |
| \`stdio\` | printf/vprintf/fprintf/vfprintf/sprintf/snprintf/vsnprintf/puts/putchar/getchar/fscanf + fopen/fclose/fread/fwrite/fflush/fgetc/fputc/fgets/fputs/feof/ferror | printf 写 fd 1；FILE 流经内核 VFS |
| \`stdio_format\` | printf 格式引擎（Spec/Conv/Length 解析、emit_int/emit_str/emit_char） | 纯逻辑，host 单测 |
| \`float\` | decompose + emit_fixed/emit_exp/emit_general（\`%f/%e/%g\`） | 纯逻辑，host 单测 |
| \`stdlib\` | abs/labs/llabs/atoi/atol/atoll/strtol/strtoul/strtoll/strtoull/strtod/strtof/strtold/rand/srand/div/ldiv/qsort/bsearch | 纯逻辑；strtod 严格正确舍入（大整数精确法），strtof 亦严格正确舍入（直接 f32 精确路径） |
| \`random\` | xorshift64* PRNG | 未播种时经 libsys 时间自播种 |
| `wchar` | wcslen/wcscmp/wcscpy/wcscat/wcschr/mbrtowc/wcrtomb/mbsrtowcs/wcsrtombs/mbstowcs/wcstombs | 宽字符（wchar_t=i32，逐字节扩展编码，无 locale） |
| \`unistd\` | open/close/read/write/lseek/unlink/chdir/getcwd/isatty | 经 libsys io 域 |
| \`process\` | exit/_exit/getpid/kill/waitpid/yield_sys | 经 libsys process 域 |
| \`time\` | time/clock/sleep/usleep/nanosleep | 经 libsys 墙钟与 sleep |
| \`errno\` | errno()/__errno_location()/set_errno + 错误码常量 | 全局 AtomicI32 |
| \`ctypes\` | size_t/ssize_t/c_int/.../NULL/EOF 等 | — |

## 设计要点

- **错误处理（ADR-010）**：所有可能失败的调用经 \`__errno_location()\` 写全局 errno 并返回
  -1/NULL；errno 值与内核 ADR-010 对齐（EINVAL=22, ENOENT=2, ENOMEM=12, ERANGE=34 等）。
- **malloc**：16 字节块头，payload 16 字节对齐；首匹配 + 分裂 + 地址序合并；自旋锁保护；；加固：canary 越界守卫、free 0xDD 毒化、double-free 检测
  堆经 \`brk\` 按 64KB 增长。与 libsys 的 Rust 全局分配器（buddy）相互独立、不冲突。
- **printf 浮点**：f64 分解为 \`d0.d1d2... × 10^dec_exp\`（**大整数精确法**，
  float_bigint.rs，全值域精确），round-half-even 舍入，inf/-inf/nan 显式输出。

## 使用方式（用户态 Rust 程序）

```toml
[dependencies]
libsys = { path = "../libsys" }
libc   = { path = "../libc" }
```

```rust
// 直接经 libc crate 路径调用其 C ABI 函数（强制链接 + 真实调用）。
unsafe {
    let p = libc::malloc::malloc(64);
    let n = libc::stdio::snprintf(buf.as_mut_ptr() as *mut i8, buf.len(),
        b"%d %.2f\0".as_ptr() as *const i8, 42, 3.14);
    libc::malloc::free(p);
}
```

> 也可用 \`unsafe extern "C" { fn malloc(size: usize) -> *mut u8; }\` 声明调用
> \`#[no_mangle]\` 符号；但建议经 crate 路径调用以确保链接期包含 libc 目标文件。

## C 头文件（libc/include/）

为未来原生 C 工具链提供头文件（staticlib 已产出）：\`boruix.h\` 聚合头，
\`string.h\` / \`stdlib.h\` / \`stdio.h\` / \`unistd.h\` / \`malloc.h\` /
\`time.h\` / \`errno.h\` / \`boruix_ctypes.h\`。链接时 \`memcpy/memset/memcmp/memmove\`
由 libsys(builtins) 满足，其余由 libc staticlib 满足。

## 端到端验证（shell）

\`shell\` 内建命令 \`libccheck\` 在真实内核上验收 malloc/string/printf/strtol/time/FILE 流：
在 shell 输入 \`libccheck\`，输出各检查项 OK/FAIL 与汇总 \`[libccheck] passed=N failed=M\`。

## host 单测（TDD）

\`libc/test_harness/\` 是一个 host std 测试 crate，经 \`#[path]\` include 纯逻辑模块
（stdio_format/float/ctype/stdlib），验证 printf 格式、浮点 dtoa、strtol 等：
```sh
cargo test --manifest-path libc/test_harness/Cargo.toml
```
64 个用例（含 round-half-even、负零、大数、指数、全范围 dtoa、qsort/bsearch、
strtod 正确舍入（含 2.2250738585072011e-308 次正规边界、min/max 次正规、溢出）、strtol 溢出/进制探测等对抗性边界）。

## 算法说明

- **fscanf**：支持 \`%d %i %u %x %o %s %c %f %e %g %n %% %[\`（含大写），长度
  修饰符 \`hh/h/l/ll/z/t/L/j\`（整数目标指针宽度随之变化；\`%f→float*\`、
  `%lf→double*`、`%Lf→long double*`、`%lc/%ls` 写宽字符）。实数经
  `strtod` 的精确路径解析（大整数 + 正确舍入），指数饱和到 inf/0。无 pushback 流限制见下。
- **qsort**：混合式快速排序（median-of-three 选主元 + 小分区插入排序收尾 + 显式
  栈），平均 O(n log n)，非稳定（C 语义）。见 \`stdlib.rs\`。

## 已知限制（诚实降级说明，S09）

1. **errno 进程局部**：单进程无线程模型下 errno 天然进程局部；未来引入进程内线程
   须改为 TLS（见 errno.rs 文档）。
2. **float 精度**：已用大整数精确法（float_bigint.rs）实现**全值域精确** dtoa
   （含 1e300 / 5e-324 次正规数），无窗口限制；性能为朴素大数除法，printf 默认精度足够。
3. **getpid**：内核暂无 SYS_TASK_GETPID，实现经解析 \`/processes/list\` 找 Running 状态进程
   启发式（单核顺序模型下成立）。\`docs/adr/033-process-identity.md\` 有完整设计。
4. **lseek(SEEK_CUR/SEEK_END)**：内核不暴露当前位置/末尾，如实返回 ENOTSUP（SEEK_SET 可用）。
5. **waitpid**：内核 \`waitpid_any\` 不返回 pid，waitpid 暂返回 -1（状态经 errno 表达）。
6. **FILE 流**：默认无缓冲直写（本内核页缓存写即落盘，缓冲期收益为零，S32 无优化无数据）。

## 符号冲突说明

\`memcpy/memmove/memset/memcmp\` 的 C ABI 符号由 **libsys（builtins）** 提供（编译器内建
路径依赖）。libc 的 \`string.rs\` 对这四个函数**不导出 no_mangle 符号**，仅作纯 Rust 别名，
避免与 libsys 及 Rust 编译器内建符号冲突（S09）。其余 \`str*/memchr\` 等均正常导出。

## 目录

- \`src/\` — 各模块实现
- \`test_harness/\` — host 单测（纯逻辑模块）
