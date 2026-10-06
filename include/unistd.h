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

/* execvp：**本系统不支持**——BORUIX 没有"替换当前进程映像"的系统调用（只有派生）。
 * 本实现**总是返回 -1**：按 PATH 找不到 → ENOENT（真实查找结果）；找到但做不了 → ENOTSUP。
 * 详见 libc/src/unistd.rs 的说明（为什么不"派生+等待+退出"来近似）。 */
int execvp(const char *file, char *const argv[]);
int chdir(const char *path);
char *getcwd(char *buf, size_t size);
int isatty(int fd);

/* exit/_Exit/abort/atexit 归 <stdlib.h>（POSIX 归属）；unistd.h 只留 _exit。 */
__attribute__((noreturn)) void _exit(int status);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_UNISTD_H */
