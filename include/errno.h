/* errno.h —— BORUIX libc 错误码（ADR-010 对齐）。 */
#ifndef _BORUIX_ERRNO_H
#define _BORUIX_ERRNO_H

#ifdef __cplusplus
extern "C" {
#endif

int *__errno_location(void);
#define errno (*__errno_location())

#define EPERM   1
#define ENOENT  2
#define EIO     5
#define ENOMEM  12
#define EACCES  13
#define EEXIST  17
#define EINVAL  22
#define ENOSPC  28
#define ERANGE  34
/* ENOSYS：功能未实现（POSIX）。本系统用它做**如实**拒绝——例如 posix_spawnattr_setflags
 * 的进程组/信号屏蔽/调度优先级旗标（本系统没有那些能力）。 */
#define ENOSYS  38

/* ---- 以下 15 个由**机械门** `libc/tools/audit_errno_sync.py` 一次列出：
 * Rust 侧（libc/src/errno.rs）**一直有**、C 头文件**一直没有** ⇒ 任何 C 程序比较它们都会
 * undeclared。真实触发：GCC 的 `fixincludes/fixlib.c:62` 用 EISDIR 编译时报 undeclared。
 * 数值取自 Rust 侧（那是单点真值；本门会持续校验两侧一致）。 */
#define E2BIG        7
#define ENOEXEC      8
#define EFAULT       14
#define EBUSY        16
#define ENOTDIR      20
#define EISDIR       21
#define ENFILE       23
#define EMFILE       24
#define ESPIPE       29
#define EROFS        30
#define ENAMETOOLONG 36
#define ENOTEMPTY    39
#define ELOOP        40
#define EILSEQ       84
#define EUCLEAN      117
#define EAGAIN  11
#define EBADF   9
#define ENOTSUP 95
/* EINTR（阻塞中的系统调用被信号打断，ADR-051）。取值与 libc/src/errno.rs 的 EINTR 同一事实。
 * 3P6-2 第二波：由 GCC 的真实报错驱动补上——libiberty 的 simple-object.c 用它判断"重试读"。 */
#define EINTR   4

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_ERRNO_H */
