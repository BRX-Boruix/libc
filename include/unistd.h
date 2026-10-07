/* unistd.h —— BORUIX libc POSIX 系统调用封装。 */
#ifndef _BORUIX_UNISTD_H
#define _BORUIX_UNISTD_H

#include "boruix_ctypes.h"
/* pid_t 由 <sys/types.h> 提供（POSIX 规定 unistd.h 暴露它）。
 * 3P6-2 第二波：wave2.c 在 BORUIX 内用 tcc 编译时对 getpid 报
 * "implicit declaration of function 'getpid'"——实现早就在（libc/src/process.rs），
 * 缺的只是**声明**。真实报错驱动，不是预猜。 */
#include "sys/types.h"

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

/* 进程/线程标识。POSIX 归属 unistd.h（实现在 libc/src/process.rs）：
 * getpid 返回**线程组组长** pid（POSIX 进程 id）；gettid 返回本线程自身 pid。
 * 单线程进程两者相等；多线程时组员 getpid==组长、gettid==自身（见 process.rs 文档）。 */
pid_t getpid(void);
pid_t gettid(void);

/* exit/_Exit/abort/atexit 归 <stdlib.h>（POSIX 归属）；unistd.h 只留 _exit。 */
__attribute__((noreturn)) void _exit(int status);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_UNISTD_H */
