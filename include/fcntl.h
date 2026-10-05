/* fcntl.h - BORUIX C 标准库：文件打开标志（3P3-2）。
 *
 * 取值与 libc/src/unistd.rs 的 O_* 常量**逐位一致**（那里是单点定义，内核经 libsys 解释）。
 * O_CLOEXEC 与 Linux 同值（0o2000000 = 0x80000）。
 */
#ifndef _FCNTL_H
#define _FCNTL_H

#include "sys/types.h"

#define O_RDONLY  0
#define O_WRONLY  1
#define O_RDWR    2
#define O_CREAT   0x40
#define O_TRUNC   0x200
#define O_APPEND  0x400
#define O_CLOEXEC 0x80000

int open(const char *path, int flags, ...);

#endif /* _FCNTL_H */