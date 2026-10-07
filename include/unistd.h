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

/* dup2：把 oldfd 复制到 newfd（POSIX）。副本与原 fd 共享文件偏移。
 * 3P6-2 第二波：由 GCC 的真实报错驱动补上（libiberty 的 filedescriptor.c 需要它）。 */
int dup2(int oldfd, int newfd);

/* ---- 以下 5 项由**头文件覆盖审计**（libc/tools/audit_header_coverage.py）一次列出：----
 * 实现在 libc/src/*.rs 里早已存在，只是此前没有任何头文件声明（与 getpid/dup2/EINTR 同类）。 */
int chown(const char *path, uid_t owner, gid_t group);
int ftruncate(int fd, off_t length);
int symlink(const char *target, const char *linkpath);
long sysconf(int name);
/* fork：COW 语义；多线程父进程被内核如实拒绝（ENOTSUP）。三条差异见 libc/src/process.rs 的文档。 */
pid_t fork(void);

/* sysconf 的 name 取值：本实现只支持 _SC_NPROCESSORS_ONLN（取值与 Linux 一致），
 * 不支持的名字如实返回 -1 置 EINVAL（**不编造**返回值）。 */
#define _SC_NPROCESSORS_ONLN 84

/* 进程/线程标识。POSIX 归属 unistd.h（实现在 libc/src/process.rs）：
 * getpid 返回**线程组组长** pid（POSIX 进程 id）；gettid 返回本线程自身 pid。
 * 单线程进程两者相等；多线程时组员 getpid==组长、gettid==自身（见 process.rs 文档）。 */
pid_t getpid(void);
pid_t gettid(void);

/* dup：复制 fd 到最低空闲槽（POSIX）。与 fcntl(fd, F_DUPFD, 0) 同实现、同语义，
 * 副本与原 fd 共享文件偏移。
 * 3P6-2 第二波「整项缺失」类：由 libc/tools/audit_posix_surface.py 的反向对账列出
 * （此前 F_DUPFD 已实现，但 POSIX 的 dup() 本身既没实现也没声明）。 */
int dup(int fd);

/* getpagesize / pathconf / mktemp：3P6-2 第二波（GCC 宿主侧构建的真实报错驱动）。
 * pathconf 的 _PC_* 取值与 Linux 一致；无定义的 name 如实返回 -1 置 EINVAL。 */
int getpagesize(void);
/* pipe：创建匿名管道（fds[0] 读端、fds[1] 写端）。内核管道早已接线，libc 只差包装。
 * brk/sbrk：传统断点接口，底层走已有的 boruix_brk。3P6-2 第二波（GCC 驱动）。 */
int pipe(int fds[2]);
/* access：可访问性检查。**诚实边界**：F_OK 走 stat；R_OK/W_OK 用 open 探一次（内核真实判定，
 * 但会真的打开文件）；X_OK 退化为看属主执行位（无原语，近似）。3P6-2 第二波（GCC 驱动）。 */
#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4
int access(const char *path, int mode);
int brk(void *addr);
void *sbrk(long incr);
#define _PC_LINK_MAX  0
#define _PC_MAX_CANON 1
#define _PC_MAX_INPUT 2
#define _PC_NAME_MAX  3
#define _PC_PATH_MAX  4
#define _PC_PIPE_BUF  5
long pathconf(const char *path, int name);

/* rmdir / truncate / getppid：3P6-2 第二波「整项缺失」类（反向对账列出）。
 * truncate 走 open+ftruncate+close（内核只有按 fd 的 ftruncate）；rmdir 复用内核 unlink
 * （它同时支持删空目录）；getppid 数据来自 procfs 快照。 */
int rmdir(const char *path);
int truncate(const char *path, off_t length);
pid_t getppid(void);

/* 身份查询（POSIX 归属 unistd.h；实现在 libc/src/process.rs，数据源 libsys::identity_query）。
 * **诚实边界**：本系统内核只维护一份 uid/gid，**没有** real/effective 之分，
 * 故 geteuid()==getuid()、getegid()==getgid()——这是事实陈述，不是占位。 */
uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);

/* 身份变更（POSIX 归属 unistd.h；实现在 libc/src/posix_batch3.rs）。
 * **诚实边界**：本内核只维护一份 uid/gid，没有 real/effective 之分，故 setuid 一次改全部；
 * 授权按 CAP_SYSTEM 二分——无该能力时只能降权或不变，否则内核如实拒绝（EPERM）。 */
int setuid(uid_t uid);
int setgid(gid_t gid);

/* pread/pwrite：显式定位读写，**不改变**文件偏移（POSIX）。底层 libsys 早已导出，只差包装。 */
ssize_t pread(int fd, void *buf, size_t count, off_t offset);
ssize_t pwrite(int fd, const void *buf, size_t count, off_t offset);

/* readlink：读符号链接目标。返回写入 buf 的字节数（不含 NUL）。
 * 3P6-2 第二波：实现早就在 libc/src/unistd.rs，漏的是 #[unsafe(no_mangle)]——
 * 于是 libc.a 里没有该符号（PHANTOM：头文件声明了、库里没有，调用即链接失败）。 */
ssize_t readlink(const char *path, char *buf, size_t bufsiz);

/* getopt：命令行短选项解析（POSIX 归属 unistd.h；实现在 libc/src/posix_batch4.rs）。
 * **诚实边界**：不做参数置换——遇到第一个非选项即停止（POSIX 允许的经典行为）。
 * getopt_long 与 struct option 归 <getopt.h>。 */
extern char *optarg;
extern int optind, opterr, optopt;
int getopt(int argc, char *const argv[], const char *optstring);

/* exit/_Exit/abort/atexit 归 <stdlib.h>（POSIX 归属）；unistd.h 只留 _exit。 */
__attribute__((noreturn)) void _exit(int status);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_UNISTD_H */
