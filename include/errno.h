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

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_ERRNO_H */
