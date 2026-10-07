/* sys/wait.h - BORUIX C 标准库：子进程等待（3P6-2 第二波，头文件覆盖审计驱动）。
 *
 * waitpid 在 libc/src/process.rs 里早已实现，而它的 POSIX 归属就是本头文件（此前**根本没有**）。
 *
 * **诚实边界（S09）**：
 *  - 内核只提供"等任意子进程"（libsys::waitpid_any）；pid > 0 的精确匹配与 pid == 0 的
 *    "同进程组"本系统都没有 -> 一律如实按"等任意子"处理（见 process.rs 的文档）。
 *  - options != 0（WNOHANG / WUNTRACED）**不支持**，如实返回 -1 置 ENOTSUP。
 *  - 状态字按 POSIX 约定打包：**退出码在高 8 位**（process.rs 明确写入），故下面的 W* 宏成立。
 *    本系统只产生"正常退出"，故 WIFEXITED 恒真、WIFSIGNALED 恒假 —— 这是**事实陈述**，
 *    不是占位（内核不产生信号终止的状态字）。
 */
#ifndef _SYS_WAIT_H
#define _SYS_WAIT_H

#include "types.h"

#define WNOHANG   1
#define WUNTRACED 2

#define WIFEXITED(s)   (((s) & 0x7f) == 0)
#define WEXITSTATUS(s) (((s) & 0xff00) >> 8)
#define WIFSIGNALED(s) (((s) & 0x7f) != 0 && ((s) & 0x7f) != 0x7f)
#define WTERMSIG(s)    ((s) & 0x7f)
#define WIFSTOPPED(s)  (((s) & 0xff) == 0x7f)

int waitpid(int pid, int *status, int options);

#endif /* _SYS_WAIT_H */
