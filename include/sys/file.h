/* sys/file.h —— BORUIX libc：BSD 风格文件锁 flock（glibc 同样把 flock 放在本头文件）。
 *
 * **诚实边界（S09，与 libc/src/unistd.rs 的同名函数逐条对应）**：
 * - 内核锁表**不阻塞**（冲突即返回 Busy）⇒ 不带 LOCK_NB 的加锁请求**如实返回 ENOTSUP**，
 *   本系统做不到"等到拿到锁"；要非阻塞语义请显式带 LOCK_NB。
 * - 带 LOCK_NB 且冲突 ⇒ EWOULDBLOCK。
 * - 锁按**调用者 uid** 登记（同一 uid 的多个进程共享同一把锁），与 Linux 的
 *   per-open-file-description 语义不同。
 */
#ifndef _BORUIX_SYS_FILE_H
#define _BORUIX_SYS_FILE_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/* 操作位取值与 glibc/Linux 一致；LOCK_NB 可与 LOCK_SH/LOCK_EX 按位或。 */
#define LOCK_SH 1
#define LOCK_EX 2
#define LOCK_NB 4
#define LOCK_UN 8

/* flock(fd, operation)：成功返回 0，失败返回 -1 并置 errno。 */
int flock(int fd, int operation);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_SYS_FILE_H */
