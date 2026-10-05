/* unistd.h —— BORUIX libc POSIX 系统调用封装。 */
#ifndef _BORUIX_UNISTD_H
#define _BORUIX_UNISTD_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/* O_* 与 open() 归 <fcntl.h>（POSIX 归属）——本头文件不再重复定义，
 * 否则与 fcntl.h 冲突（实测：宏重定义 + open 原型冲突）。 */

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

int close(int fd);
ssize_t read(int fd, void *buf, size_t count);
ssize_t write(int fd, const void *buf, size_t count);
long lseek(int fd, long offset, int whence);
int unlink(const char *path);
int chdir(const char *path);
char *getcwd(char *buf, size_t size);
int isatty(int fd);

/* exit/_Exit/abort/atexit 归 <stdlib.h>（POSIX 归属）；unistd.h 只留 _exit。 */
__attribute__((noreturn)) void _exit(int status);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_UNISTD_H */
