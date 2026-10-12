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

// `cargo test -p libc`（宿主单测）跑不通的**真实**原因（逐条实测得出，勿再凭推断）：
//
// 1. **符号与宿主 CRT 冲突**（真障碍，也是唯一剩下的）：本 crate 用 `#[no_mangle]` 导出
//    `atexit` 等 C 符号，而宿主 msvcrt.lib 也定义它们 ⇒ 链接期
//    `LNK2005: atexit 已经在 ... 中定义` + `LNK1169: 找到一个或多个多重定义的符号`。
//    **任何 crate 拆分都解决不了**——符号在本体里。
// 2. 目标专用的 `global_asm!`（x87 long double、setjmp）会进宿主测试构建 ⇒
//    `expected absolute expression` / `unknown directive`。**已按目标门禁修掉**（见下方 `longdouble`）。
//
// 【曾经写错的判断，留档以免重蹈】此前这里写着「`staticlib` crate-type 要求宿主完整链接」
// 与「`panic=abort` 与 cargo test 冲突」——**两条都不成立**：
//   - `cargo test --lib` 只构建测试目标，`no global memory allocator` / `#[panic_handler]` /
//     `unwinding panics are not supported` 三条**全部消失**（实测）；
//   - `panic` 对 test profile 只是被 Cargo **忽略并警告**，不是错误。
//
// **本项目的既有做法**（见 tools/checks/math_verify/verify.rs）：把**纯逻辑**留在可 `include!`
// 的源文件里，宿主侧用 `mod x { include!("...") }` 引入**同一份源码**做逐点验证。
// 故 `mode.rs` 这类纯模块应走 `tools/checks/` 下的宿主验证器，而不是拆 crate。

pub mod ctype;
pub mod ctypes;
pub mod dirent;
pub mod errno;
pub mod exec;
pub mod float;
mod float_bigint;
// long double（x87 80 位）运算：Rust 无 f80 类型，只能走 global_asm。
pub mod libgen;
pub mod locale;
pub mod glob;
pub mod posix_batch2;
pub mod posix_batch3;
pub mod posix_batch4;
pub mod posix_batch5;
pub mod identity;
pub mod mode;
pub mod posix_batch6;
pub mod posix_batch7;
pub mod resource;
pub mod semaphore;
pub mod math_core;
pub mod math_core2;
// qsort 的纯计算核心（宿主对照验证用；见 qsort_core.rs 文件头记的那处越界读缺陷）。
pub mod qsort_core;
// 数学函数的 C ABI 导出层（核心保持纯函数，便于宿主对照验证器 include!）。
pub mod math_exports;
pub mod regex;
// **目标门禁**：本模块与 `setjmp` 是 x86-64 **裸机**汇编（x87 80 位 long double、非局部跳转），
// 宿主（Windows/COFF）汇编器不认这些指令与指示符 ⇒ 进宿主测试构建会报
// `expected absolute expression` / `unknown directive`。按目标门禁**本来就该做**：
// 这段汇编与宿主无关，不是为测试而打的补丁。
#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "boruix")))]
pub mod longdouble;
pub mod math_classify;
pub mod math_decomp;
pub mod malloc;
pub mod mman;
pub mod process;
pub mod pwd;
pub mod random;
#[cfg(all(target_arch = "x86_64", any(target_os = "none", target_os = "boruix")))]
pub mod setjmp;
pub mod sha256;
pub mod shadow;
pub mod signal;
pub mod spawn;
pub mod stdio;
pub mod stdio_format;
pub mod stdlib;
pub mod string;
pub mod thread;
pub mod time;
pub mod unistd;
pub mod wchar;
pub mod wcwidth;

/// 初始化 libc（标准流等）。幂等，可安全多次调用。
#[unsafe(no_mangle)]
pub extern "C" fn libc_initialize() {
    stdio::stdio_init();
}
