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
/* O_EXCL（3P6-2 第二波）：与 O_CREAT 同用时独占创建，文件已存在则失败。
 * 取值与 Linux 一致（0o200）。原子性由内核 sys_open 保证，非用户态两步近似。 */
#define O_EXCL    0x80
#define O_CREAT   0x40
#define O_TRUNC   0x200
#define O_APPEND  0x400
#define O_CLOEXEC 0x80000

/* fcntl 命令常量（x86_64 Linux ABI，与 libc/src/unistd.rs 的 F_* 同一事实）。
 * **来路（3P6-2 第二波，真实报错驱动）**：wave2.c 在系统内用 tcc 编译时报
 *   error: 'F_DUPFD' undeclared  /  warning: implicit declaration of function 'fcntl'
 * ——实现早就在（libc/src/unistd.rs 的 fcntl），但 <fcntl.h> **只声明了 open**，
 * fcntl 只在别处的**注释**里被提到过（这正是"名字审计"的假阴性：注释也算"提到"）。 */
#define F_DUPFD  0
#define F_GETFD  1
#define F_SETFD  2
#define F_GETFL  3
#define F_SETFL  4
/* FD_CLOEXEC：fd 标志。本内核无 exec 关闭语义 → F_SETFD 非 0 时如实拒绝（见 unistd.rs）。 */
#define FD_CLOEXEC 1

int open(const char *path, int flags, ...);
/* fcntl(fd, cmd, ...)：fd 控制。真实支持 F_DUPFD（复制到 >= arg 的最低空闲槽）；
 * F_GETFD 恒 0、F_SETFD(0) 恒 0，其余命令如实返回 -1 置 ENOTSUP（**不伪造**）。 */
int fcntl(int fd, int cmd, ...);

#endif /* _FCNTL_H */