# libc

**简体中文** | [English](#english)

BORUIX 的 **C 标准库**——用 Rust 实现、对外提供标准 C 接口。

程序可以用 C 写、调用熟悉的 `printf`、`malloc`、`strlen`，而底层实现是内存安全的 Rust。

---

## 这是什么

一个操作系统的价值很大程度上取决于它能跑什么，而大量的软件是用 C 写的。`libc` 就是要让这些
软件能在这个系统上编译和运行。

它的定位是**接口与实现的分离**：C 库是一套**接口约定**（`printf`、`malloc` 这些符号的签名
与语义），至于用什么语言实现是自由的。这里选择用 Rust 实现，因为 Rust 能精确导出 C 接口，同时
让内部实现享有内存安全。

## 设计思路

BORUIX 的程序绝大部分是 Rust 写的，因此这个库有**两类使用者**：

| 使用者 | 调用方式 |
| --- | --- |
| Rust 程序 | 直接经由 Rust 路径调用 |
| C 程序 | 经由标准 C 接口调用 |

两者由**同一份实现**支撑。这不是两套代码，而是同一套函数同时以 Rust 和 C 两种方式暴露。

具体做法是：所有函数用 `extern "C"` 导出，同时提供配套的 C 头文件。Rust 程序可以直接调用，
C 程序可以链接静态库并包含头文件。

## 已实现的功能

覆盖 C 标准库的主要部分：

| 领域 | 内容 |
| --- | --- |
| 内存管理 | `malloc` / `free` / `realloc` / `calloc` / 对齐分配 |
| 字符串与内存块 | `memcpy` / `strlen` / `strcmp` / `strtok` / `strstr` 等 |
| 格式化输出 | `printf` / `snprintf` / `fprintf` 及完整的格式引擎 |
| 格式化输入 | `fscanf` 系列 |
| 文件流 | `fopen` / `fread` / `fwrite` / `fgets` 等 |
| 数值转换 | `strtol` / `strtod` / `atoi` 等，**严格正确舍入** |
| 字符分类 | `isalpha` / `tolower` 等 |
| 排序查找 | `qsort` / `bsearch` |
| 系统调用封装 | `open` / `read` / `write` / `lseek` / `getcwd` 等 |
| 进程与时间 | `exit` / `getpid` / `waitpid` / `time` / `nanosleep` |
| 宽字符 | `wcslen` / `mbrtowc` / `wcstombs` 等 |
| 错误处理 | `errno` 及标准错误码 |
| 线程 | 线程创建、汇合与同步 |

用 C 写程序的开发者会发现这些都是熟悉的东西。

## 值得说明的实现

**浮点格式化是全值域精确的。** 把浮点数转成十进制文本（`%f` / `%e` / `%g`）看起来简单，
实则容易出错——朴素的算法在极端数值上会产生最后一位的偏差。这里的实现使用精确的大整数运算，
在**全部数值范围内**保证结果正确，包括次正规数这类边界情况，并采用银行家舍入（round-half-even，
与标准一致）。

**字符串转浮点同样严格正确舍入。** 把文本解析成浮点数（`strtod`）是同一个问题的逆过程，同样
容易在边界上出错。实现走同一套精确路径，覆盖次正规数边界与溢出情形。

**内存分配器带加固。** 除了基本的分配释放，还包含越界守卫、释放后填充标记、以及重复释放检测——
这些能让常见的内存使用错误在发生时暴露，而不是变成难以追踪的随机故障。

## 已知限制

与标准 C 库相比，有几处**如实说明**的差异：

| 限制 | 说明 |
| --- | --- |
| `errno` 是进程级的 | 当前无线程模型下天然如此 |
| `lseek` 只支持绝对定位 | 相对当前位置与文件末尾的定位不支持，会如实返回"操作不支持" |
| `waitpid` 的部分选项 | 只支持等待任意子进程，其余选项如实返回错误 |
| 文件流默认无缓冲 | 直接写到底层，因为该层的缓冲带不来收益 |
| 无区域设置 | 宽字符按固定编码处理 |

这些不是实现疏漏，而是当前系统能力下的**有意取舍**。使用前请确认你的程序不依赖这些行为。

## 使用

### 从 Rust 程序使用

```toml
[dependencies]
libc = { path = "../libc" }
```

```rust
unsafe {
    let p = libc::malloc::malloc(64);
    libc::stdio::snprintf(buf.as_mut_ptr() as *mut i8, buf.len(),
        b"%d %.2f\0".as_ptr() as *const i8, 42, 3.14);
    libc::malloc::free(p);
}
```

### 从 C 程序使用

包含 `include/` 下的头文件并链接本库产出的静态库即可。聚合头文件是 `boruix.h`。

## 测试

逻辑部分（格式化引擎、浮点转换、数值解析等）经过约 **64 个单元测试**，覆盖边界与对抗性情形：
银行家舍入、负零、极端指数、次正规数边界、数值溢出、进制探测等。

此外系统 shell 内有一个端到端检查命令，在真实内核上验证内存分配、字符串、格式化输出、数值转换
和时间等链路。

## 构建

```bash
cargo build --release
cargo test --manifest-path test_harness/Cargo.toml
```

## 目录

```
libc/
├── include/        # C 头文件
├── src/            # 各模块实现
└── test_harness/   # 逻辑部分的单元测试
```

## 相关项目

- [`libsys`](https://github.com/BRX-Boruix/libsys) —— 用户态系统调用封装
- [`csrc`](https://github.com/BRX-Boruix/csrc) —— 自由式 C 运行环境

## 许可

MIT License，版权归 Yang Borui 所有。详见 [LICENSE](LICENSE)。

---

# English

[简体中文](#libc) | **English**

BORUIX's **C standard library** — implemented in Rust, exposing standard C interfaces.

Programs can be written in C, calling familiar `printf`, `malloc`, and `strlen`, while the
implementation underneath is memory-safe Rust.

---

## What this is

An operating system's value depends heavily on what it can run, and a great deal of software is
written in C. `libc` exists so that such software can be compiled and run on this system.

Its premise is the **separation of interface from implementation**: a C library is a set of
**interface contracts** (the signatures and semantics of symbols like `printf` and `malloc`), and
the language used to implement them is free. This one is implemented in Rust, because Rust can export
C interfaces precisely while giving the implementation memory safety.

## Design approach

Nearly all BORUIX programs are written in Rust, so this library has **two kinds of consumer**:

| Consumer | How it calls in |
| --- | --- |
| Rust programs | directly, through the Rust path |
| C programs | through the standard C interfaces |

Both are served by **one implementation**. This is not two codebases — it is one set of functions
exposed both as Rust and as C.

Concretely, every function is exported with `extern "C"` and accompanied by matching C headers.
Rust programs can call them directly; C programs can link the static library and include the headers.

## What is implemented

The main body of the C standard library is covered:

| Area | Contents |
| --- | --- |
| Memory management | `malloc` / `free` / `realloc` / `calloc` / aligned allocation |
| Strings and memory blocks | `memcpy` / `strlen` / `strcmp` / `strtok` / `strstr` and more |
| Formatted output | `printf` / `snprintf` / `fprintf` with a complete format engine |
| Formatted input | the `fscanf` family |
| File streams | `fopen` / `fread` / `fwrite` / `fgets` and more |
| Numeric conversion | `strtol` / `strtod` / `atoi` and more, **correctly rounded** |
| Character classification | `isalpha` / `tolower` and more |
| Sorting and searching | `qsort` / `bsearch` |
| Syscall wrappers | `open` / `read` / `write` / `lseek` / `getcwd` and more |
| Processes and time | `exit` / `getpid` / `waitpid` / `time` / `nanosleep` |
| Wide characters | `wcslen` / `mbrtowc` / `wcstombs` and more |
| Error handling | `errno` and the standard error codes |
| Threads | thread creation, joining, and synchronisation |

Developers writing C will find these familiar.

## Implementation notes worth stating

**Floating-point formatting is exact across the full range.** Turning a float into decimal text
(`%f` / `%e` / `%g`) looks simple but is easy to get wrong — naive algorithms produce last-digit
errors at extreme values. This implementation uses exact big-integer arithmetic to guarantee correct
results **across the entire numeric range**, including subnormals, with round-half-even rounding as
the standard requires.

**String-to-float parsing is likewise correctly rounded.** Parsing text into a float (`strtod`) is
the same problem in reverse and equally prone to boundary errors. It takes the same exact path,
covering subnormal boundaries and overflow.

**The allocator carries hardening.** Beyond basic allocation and freeing, it includes overrun guards,
post-free fill marking, and double-free detection — so common memory-use errors surface where they
happen rather than turning into unreproducible random failures.

## Known limitations

There are a few **honestly stated** differences from a standard C library:

| Limitation | Explanation |
| --- | --- |
| `errno` is process-wide | Naturally so given the current lack of a thread model |
| `lseek` supports absolute positioning only | Seeking relative to the current position or end of file is unsupported and returns "operation not supported" honestly |
| Some `waitpid` options | Only waiting for any child is supported; other options return an error honestly |
| File streams are unbuffered by default | They write straight through, since buffering at that layer buys nothing |
| No locale support | Wide characters use a fixed encoding |

These are not oversights but **deliberate trade-offs** under the system's current capabilities.
Check that your program does not rely on them before use.

## Usage

### From a Rust program

```toml
[dependencies]
libc = { path = "../libc" }
```

```rust
unsafe {
    let p = libc::malloc::malloc(64);
    libc::stdio::snprintf(buf.as_mut_ptr() as *mut i8, buf.len(),
        b"%d %.2f\0".as_ptr() as *const i8, 42, 3.14);
    libc::malloc::free(p);
}
```

### From a C program

Include the headers under `include/` and link the static library this crate produces. The umbrella
header is `boruix.h`.

## Testing

The logic portion (the format engine, float conversion, numeric parsing, and so on) is covered by
roughly **64 unit tests** spanning boundary and adversarial cases: round-half-even, negative zero,
extreme exponents, subnormal boundaries, numeric overflow, and base detection.

In addition, a system shell command performs an end-to-end check on real hardware, exercising memory
allocation, strings, formatted output, numeric conversion, and time.

## Building

```bash
cargo build --release
cargo test --manifest-path test_harness/Cargo.toml
```

## Layout

```
libc/
├── include/        # C headers
├── src/            # module implementations
└── test_harness/   # unit tests for the logic portion
```

## Related projects

- [`libsys`](https://github.com/BRX-Boruix/libsys) — the user-space syscall wrapper
- [`csrc`](https://github.com/BRX-Boruix/csrc) — the freestanding C runtime

## License

MIT License, copyright Yang Borui. See [LICENSE](LICENSE).
