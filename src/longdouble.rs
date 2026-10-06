//! `long double`（x87 80 位）运算——阶段 6 / 3P6-2。
//!
//! **为什么必须用汇编**：本目标的 `long double` 是 **x87 80 位扩展精度**（实测 16 字节，
//! 见 tcc-on-boruix/boruix/_probe_types.c 的同类探针思路）。Rust **没有稳定的 `f80` 类型**，
//! 无法用 Rust 代码表达这个宽度，故用 `global_asm!` 直接走 x87 指令。
//!
//! # SysV AMD64 的 long double 调用约定
//! - 参数属 **MEMORY 类**：`long double` 实参经**栈**传递（不是寄存器）；
//! - 返回值在 **`st0`**。
//! 故入口处 `[rsp+8]` 起 16 字节是 `x`，`exp` 在 `edi`。

use core::arch::global_asm;

global_asm!(
    r#"
.section .text
.global ldexpl
.type ldexpl, @function
ldexpl:
    push rbp
    mov rbp, rsp
    sub rsp, 16
    mov [rbp - 8], edi          /* exp（32 位） */
    fild dword ptr [rbp - 8]    /* st0 = exp */
    fld tbyte ptr [rbp + 16]    /* st0 = x, st1 = exp */
    fscale                      /* st0 = x * 2^exp */
    fstp st(1)                  /* 弹出 exp 操作数，结果留在 st0 */
    leave
    ret
.size ldexpl, . - ldexpl
"#
);