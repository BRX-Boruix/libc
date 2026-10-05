//! `setjmp` / `longjmp`（C ABI，3P3-2）。
//!
//! **为什么用汇编**：`longjmp` 必须把**返回地址与栈指针**一起恢复——这在 Rust 里无法表达
//! （Rust 没有非局部跳转）。故用 `global_asm!` 提供两个符号，与 C 侧约定一致。
//!
//! x86-64 的 `jmp_buf` 布局（本实现约定，`setjmp.h` 用 `long[8]` 表达同一件事）：
//! ```text
//!   [0]=rbx [8]=rbp [16]=r12 [24]=r13 [32]=r14 [40]=r15 [48]=rsp [56]=rip
//! ```
//! 只保存 **callee-saved** 寄存器（调用约定保证其余寄存器由调用方负责）。
//!
//! `setjmp` 直接调用返回 0；`longjmp` 使 `setjmp` 看起来返回 `val`（`val == 0` 时按 C 语义
//! 改为 1，否则调用方无法区分「直接返回」与「longjmp 回来」）。
//!
//! **诚实边界**：不保存信号屏蔽字（那是 `sigsetjmp`/`siglongjmp` 的语义，本系统尚未提供）。

use core::arch::global_asm;

global_asm!(
    r#"
.section .text
.global setjmp
.type setjmp, @function
setjmp:
    mov [rdi + 0], rbx
    mov [rdi + 8], rbp
    mov [rdi + 16], r12
    mov [rdi + 24], r13
    mov [rdi + 32], r14
    mov [rdi + 40], r15
    lea rax, [rsp + 8]
    mov [rdi + 48], rax
    mov rax, [rsp]
    mov [rdi + 56], rax
    xor eax, eax
    ret

.global longjmp
.type longjmp, @function
longjmp:
    mov rbx, [rdi + 0]
    mov rbp, [rdi + 8]
    mov r12, [rdi + 16]
    mov r13, [rdi + 24]
    mov r14, [rdi + 32]
    mov r15, [rdi + 40]
    mov rsp, [rdi + 48]
    mov rax, rsi
    test rax, rax
    jnz 1f
    mov rax, 1
1:  jmp qword ptr [rdi + 56]
.size setjmp, . - setjmp
.size longjmp, . - longjmp
"#
);