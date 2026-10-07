/* setjmp.h - BORUIX C 标准库：非局部跳转（3P3-2）。
 *
 * `jmp_buf` 是 8 个 long（64 字节）：rbx/rbp/r12/r13/r14/r15/rsp/rip。
 * 实现见 libc/src/setjmp.rs（汇编，因 longjmp 必须恢复返回地址与栈指针）。
 *
 * `sigjmp_buf` 比 `jmp_buf` 多两格（[8] = 屏蔽集、[9] = 是否保存过的标记）。
 * 两格**必须在类型上分开**：把屏蔽集塞进 `jmp_buf` 会让既有 setjmp 用户越界写。
 *
 * **诚实边界**：`sigsetjmp(env, 0)` 不碰屏蔽集（POSIX 允许），此时 `siglongjmp` 也不恢复它——
 * 用显式标记区分「保存过 0」与「没保存」，而不是拿 0 当哨兵（屏蔽集为 0 是合法状态）。
 */
#ifndef _SETJMP_H
#define _SETJMP_H

typedef long jmp_buf[8];
typedef long sigjmp_buf[10];

int setjmp(jmp_buf env);
__attribute__((noreturn)) void longjmp(jmp_buf env, int val);

/* sigsetjmp(savemask != 0) 额外保存进程信号屏蔽集，siglongjmp 恢复它。
 * 实现见 libc/src/setjmp.rs：汇编**尾跳** setjmp——若在中间函数里调用 setjmp，
 * 保存的会是那个函数的帧，longjmp 回去时它已不存在。 */
int sigsetjmp(sigjmp_buf env, int savemask);
__attribute__((noreturn)) void siglongjmp(sigjmp_buf env, int val);

#endif /* _SETJMP_H */