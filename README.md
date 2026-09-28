# libc

BORUIX 的 C 标准库：用 Rust 实现，导出标准 C 接口，构建于 [`libsys`](https://github.com/BRX-Boruix/libsys) 之上。

[English](README.en.md)

`include/` 下提供对应的 C 头文件，聚合头为 `boruix.h`。

## 已实现

- 内存管理——`malloc`、`free`、`realloc`、`calloc`、`posix_memalign`、`aligned_alloc`
- 字符串与内存块——`memcpy`、`strlen`、`strcmp`、`strstr`、`strtok_r` 等
- 格式化输出——`printf`、`snprintf`、`fprintf`、`vprintf` 及完整格式引擎（含 `%f`、`%e`、`%g`）
- 格式化输入——`fscanf` 系列
- 文件流——`fopen`、`fread`、`fwrite`、`fgets`、`fputs`、`fflush` 等
- 数值转换与排序——`strtol`、`strtod`、`atoi`、`qsort`、`bsearch`
- 字符分类——`isalpha`、`tolower`、`toupper` 等
- 系统调用封装——`open`、`read`、`write`、`lseek`、`unlink`、`chdir`、`getcwd`、`isatty`
- 进程与时间——`exit`、`getpid`、`kill`、`waitpid`、`time`、`nanosleep`
- 宽字符——`wcslen`、`mbrtowc`、`wcstombs` 等
- 线程——每线程控制块与线程本地存储，`errno` 按线程隔离

## 使用

**Rust 程序**：作为依赖引入（crate 类型为 rlib 与 staticlib）：

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

**C 程序**：包含 `include/` 下的头文件，链接本库产出的静态库。

## 已知限制

- `errno` 的线程隔离要求线程经本库的引导派生；未经引导的线程共享一个后备槽
- `lseek` 只支持绝对定位，`SEEK_CUR` 与 `SEEK_END` 返回 `EINVAL`
- `waitpid` 只支持等待任意子进程，其余选项返回错误
- 文件流默认无缓冲，直接读写底层
- 无 locale，宽字符按固定编码处理

## 构建与测试

```bash
cargo build --release
cargo test
cargo test --manifest-path test_harness/Cargo.toml
```

库内 19 个单元测试；`test_harness/` 是宿主机侧的测试程序，含 64 个测试函数，覆盖格式化引擎、
数值转换、排序与查找。

## 目录

```
libc/
├── include/        # C 头文件
├── src/            # 各模块实现
└── test_harness/   # 宿主机侧测试
```

## 相关项目

- [`libsys`](https://github.com/BRX-Boruix/libsys) —— 用户态系统调用封装
- [`csrc`](https://github.com/BRX-Boruix/csrc) —— 独立式 C 运行环境

## 许可

MIT License，版权归 Yang Borui 所有。详见 [LICENSE](LICENSE)。
