/* setjmp.h - BORUIX C 标准库：非局部跳转（3P3-2）。
 *
 * `jmp_buf` 是 8 个 long（64 字节）：rbx/rbp/r12/r13/r14/r15/rsp/rip。
 * 实现见 libc/src/setjmp.rs（汇编，因 longjmp 必须恢复返回地址与栈指针）。
 *
 * **诚实边界**：不保存信号屏蔽字——那是 sigsetjmp/siglongjmp 的语义，本系统尚未提供。
 */
#ifndef _SETJMP_H
#define _SETJMP_H

typedef long jmp_buf[8];

int setjmp(jmp_buf env);
__attribute__((noreturn)) void longjmp(jmp_buf env, int val);

#endif /* _SETJMP_H */