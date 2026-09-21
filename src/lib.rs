//! BORUIX libc —— 用户态 C 标准库（Rust 实现 + C ABI，构建于 libsys 之上）。
//!
//! 架构（ADR-001：Rust 实现 + C ABI）：
//! ```text
//! kernel(sycall) ← libsys(薄封装) ← libc(本 crate, C ABI) ← 用户程序
//! ```
//!
//! 本 crate 提供 C ABI（\`extern "C"\`）函数，供 no_std Rust bin crate 经
//! \`unsafe extern "C"\` 声明调用，也可产出 staticlib 供未来原生 C 工具链链接。
//! 用户程序入口（\`_start\` → \`user_main\`）仍由 libsys 提供，libc 不接管入口。
//!
//! ## 模块
//! - \`malloc\`：malloc/free/realloc/calloc（brk 堆分配器）
//! - \`string\`：mem/str 函数
//! - \`ctype\`：字符分类/转换
//! - \`stdio\`：printf 家族 + FILE* 流
//! - \`stdio_format\`：printf 格式引擎（纯逻辑）
//! - \`float\`：f64 十进制格式化（%f/%e/%g）
//! - \`stdlib\`：strtol/atoi/abs/rand/div
//! - \`unistd\`：open/close/read/write/lseek/mkdir/remove/fcntl
//! - \`dirent\`：opendir/readdir/closedir
//! - \`signal\`：signal/sigaction/sigprocmask/raise
//! - \`process\`：exit/getpid/kill/waitpid
//! - \`time\`：time/clock/sleep
//! - \`errno\`：errno 机制与错误码
//! - \`ctypes\`：C 基础类型
//! - \`random\`：PRNG（rand 用）
//!
//! ## 入口初始化
//!
//! 用户程序在 \`user_main\` 中调用任何 stdio/printf 函数前，可调用
//! \`libc_initialize()\` 初始化标准流；printf/puts 等会惰性自初始化，故显式
//! 调用可选。malloc/string 等纯内存函数无需初始化。

// 目标裸机恒 no_std；host 单测（cargo test）时用 std 提供 allocator/panic。
#![cfg_attr(not(test), no_std)]
#![feature(c_variadic)]
#![allow(non_camel_case_types)]
#![allow(unsafe_op_in_unsafe_fn)]

extern crate alloc;

pub mod ctype;
pub mod ctypes;
pub mod dirent;
pub mod errno;
pub mod float;
mod float_bigint;
pub mod malloc;
pub mod process;
pub mod pwd;
pub mod random;
pub mod sha256;
pub mod shadow;
pub mod signal;
pub mod stdio;
pub mod stdio_format;
pub mod stdlib;
pub mod string;
pub mod thread;
pub mod time;
pub mod unistd;
pub mod wchar;

/// 初始化 libc（标准流等）。幂等，可安全多次调用。
#[unsafe(no_mangle)]
pub extern "C" fn libc_initialize() {
    stdio::stdio_init();
}
