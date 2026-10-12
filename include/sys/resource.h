/* sys/resource.h —— BORUIX libc 的调度优先级（POSIX 子集）。
 *
 * **诚实边界**：本系统无进程组、无多用户（ADR-043）⇒ 只实现 PRIO_PROCESS。
 * PRIO_PGRP/PRIO_USER 的取值仍如实定义（POSIX 规定），但 getpriority/setpriority
 * 对它们返回 -1 置 EINVAL——不假装支持。
 *
 * POSIX 关系：priority = 20 - nice ⇒ getpriority 返回 20-nice（值域 1..=40）。
 * 内核侧对应 SYS_TASK_NICE（0x3F）：a1=方向、a2=pid（0=自身）、a3=新值；
 * 返回值编码为 nice+20（nice 可为负，而 syscall 错误编码占用 bit63）。
 */
#ifndef _BORUIX_SYS_RESOURCE_H
#define _BORUIX_SYS_RESOURCE_H

#include "../boruix_ctypes.h"

#define PRIO_PROCESS 0
#define PRIO_PGRP    1
#define PRIO_USER    2

#ifdef __cplusplus
extern "C" {
#endif

/* 成功返回优先级（20-nice）；which 不支持或进程不存在 -> -1 置 errno。 */
int getpriority(int which, int who);
/* 成功返回 0；改他人需 CAP_SYSTEM，越权如实 EPERM。 */
int setpriority(int which, int who, int prio);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_SYS_RESOURCE_H */