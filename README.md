# BORUIX libc

BORUIX 系统的用户态 C 标准库。

## 职责
- 提供用户态标准库接口：内存分配（`malloc`）、格式化输出（`printf`）、文件/IO、字符串等
- 建立在 `libsys`（系统调用封装层）之上
- 是用户态程序运行的基础，可独立替换实现

## 依赖关系
```
libsys  ←  (无依赖)
  ▲
libc   ←  libsys
```
每个用户态程序（`base`、`init`、`shell` 等）都依赖 `libc`。

## 内容规划
- `src/`  — C 库实现
- 对 `libsys` 的系统调用进行封装，暴露更友好的高层 API
