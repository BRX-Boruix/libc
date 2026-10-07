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
//! ## `sigsetjmp` / `siglongjmp`（C1 清单补齐）
//!
//! `sigjmp_buf` 比 `jmp_buf` 多两格：`[8]` = 保存的屏蔽集，`[9]` = 魔数标记（是否真的保存过）。
//! 多出来的两格**必须**在类型上分开——把屏蔽集塞进 `jmp_buf` 会让既有 `setjmp` 用户越界写。
//!
//! `sigsetjmp` 的实现难点是**栈帧**：`setjmp` 保存的是**调用者**的 rsp/rip，若在 Rust 函数里
//! 调用 `setjmp`，保存的就是那个中间函数的帧——`longjmp` 回去时该帧已不存在。故这里用汇编
//! **尾跳**：先 `call` 一个 Rust 助手保存屏蔽集，再 `jmp setjmp`。尾跳让栈回到入口状态，
//! `setjmp` 于是保存到**原始调用者**的帧（正确语义）。
//!
//! **诚实边界**：`savemask == 0` 时不碰屏蔽集（POSIX 允许），`siglongjmp` 也不会去恢复它。

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

/// `sigjmp_buf` 里屏蔽集标记的魔数（"SIGJ"）。为什么需要它：POSIX 允许 `sigsetjmp(env, 0)`
/// **不**保存屏蔽集，此时 `siglongjmp` **不得**恢复（否则会把调用方当前的屏蔽集改掉）。
/// 用一个显式标记区分「保存过 0」与「没保存」，比用 0 当哨兵可靠（屏蔽集为 0 是合法状态）。
pub const SIGJMP_MAGIC: crate::ctypes::c_long = 0x5349_474A;

/// `sigsetjmp` 的屏蔽集保存助手（由汇编尾跳前 `call`）。
///
/// `mask(SIGNAL_BLOCK, 0)` 是**无副作用查询**：屏蔽集不变，只回写旧值——与
/// `sigprocmask(set = NULL)` 用的是同一条内核路径（S15 单点）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sigsetjmp_prepare(env: *mut crate::ctypes::c_long, savemask: crate::ctypes::c_int) {
    if env.is_null() {
        return;
    }
    if savemask != 0 {
        let old = libsys::signal::mask(libsys::signal::SIGNAL_BLOCK, 0).unwrap_or(0);
        *env.add(8) = old as crate::ctypes::c_long;
        *env.add(9) = SIGJMP_MAGIC;
    } else {
        *env.add(9) = 0;
    }
}

unsafe extern "C" {
    /// 汇编符号 `longjmp`（本模块 `global_asm!` 提供）。
    #[link_name = "longjmp"]
    fn longjmp_sym(env: *mut crate::ctypes::c_long, val: crate::ctypes::c_int) -> !;
}

/// `siglongjmp(env, val)`：非局部跳转并**恢复** `sigsetjmp` 保存的屏蔽集（若当时保存过）。
///
/// 恢复发生在跳转**之前**：POSIX 要求 `siglongjmp` 返回后屏蔽集已是保存时的值。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn siglongjmp(env: *mut crate::ctypes::c_long, val: crate::ctypes::c_int) -> ! {
    if !env.is_null() && *env.add(9) == SIGJMP_MAGIC {
        let _ = libsys::signal::mask(libsys::signal::SIGNAL_SET, *env.add(8) as u64);
    }
    longjmp_sym(env, val)
}

// `sigsetjmp` 本体：先 `call __sigsetjmp_prepare` 保存屏蔽集，再**尾跳** `setjmp`。
// 栈对齐：入口 rsp ≡ 8 (mod 16)；push×2 后 ≡ 8；`sub rsp, 8` 后 ≡ 0，满足 call 的 ABI 要求。
global_asm!(
    r#"
.section .text
.global sigsetjmp
.type sigsetjmp, @function
sigsetjmp:
    push rdi
    push rsi
    sub rsp, 8
    call __sigsetjmp_prepare
    add rsp, 8
    pop rsi
    pop rdi
    jmp setjmp
.size sigsetjmp, . - sigsetjmp
"#
);