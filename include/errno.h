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
