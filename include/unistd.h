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

/* confstr：取配置定义字符串（POSIX）。返回值=所需长度（含 NUL）；name 无值时返回 0 置 EINVAL。
 * 已定义 _CS_PATH(1) 与 _CS_GNU_LIBC_VERSION(2)——后者如实标注是 boruix 而非 glibc。 */
size_t confstr(int name, char *buf, size_t len);
/* _CS_PATH 的取值必须与 shell/src/env.rs 的 DEFAULT_PATH 一致（两处真值源，违反 S13 单点；
 * 由 tools/checks/dist/confstr_path_agrees.py 门禁保证不漂移）。 */
#define _CS_PATH 1
#define _CS_GNU_LIBC_VERSION 2

/* getgroups：查询调用进程的补充组集合（POSIX）。
 * size == 0（list 可为 NULL）⇒ 只返回所需条数；否则写入至多 size 个并返回实际条数；
 * size 不足 ⇒ 返回 -1 置 EINVAL（不截断、不写半个列表）。
 * 内核侧对应 SYS_TASK_GROUPS_GET（0x3E）——此前只有 GROUPS_SET、无查询入口，故本项曾登记为缺失。 */
int getgroups(int size, gid_t list[]);

/* fsync：把数据冲刷到稳定存储（POSIX）。成功 0，失败 -1 置 errno。
 * 诚实边界：本内核块缓存是每设备一份 ⇒ 实现冲刷全部设备缓存（POSIX 要求的超集，粒度粗但语义正确）；
 * 仍有脏块未落盘时如实返回 -1 置 EIO（与关机路径同一条判据），不假装成功。 */
int fsync(int fd);

/* fchown：按 fd 改属主（POSIX）。uid/gid 传 -1 表示保持不变；易他主需 CAP_SYSTEM。
 * 诚实边界：本系统没有 fd→路径 解析，故由内核提供 fd 版动词（SYS_STREAM_FCHOWN 0x1C）。 */
int fchown(int fd, uid_t uid, gid_t gid);

/* link：创建硬链接（POSIX）。成功 0，失败 -1 置 errno。
 * 与 symlink 本质不同：软链接造新节点，硬链接让**同一个节点**多一个目录项
 * （两路径共享内容、st_ino 相同、st_nlink 计数）。
 * 诚实边界：目前只有 RamFS 实现（根文件系统）；ext2 上如实返回 ENOTSUP——不近似
 * （复制内容不是硬链接）。内核侧对应 SYS_ENTRY_LINK（0x49）。 */
int link(const char *target, const char *link_path);

/* nice：把调用进程的 nice 加上 inc，返回**新的** nice（POSIX）。
 * 返回值可为 -1（新的 nice 恰好为 -1 时）——POSIX 承认这一歧义，调用方须先清零 errno。
 * 越界由内核钳制到 [-20,19]（与调度器单点一致），不是报错。 */
int nice(int inc);

/* alarm：设置/取消一次性闹钟（POSIX）。返回上一个闹钟的剩余秒数（向上取整）；0 取消。
 * 到期由内核投递 SIGALRM。诚实边界：内核实现的是每线程闹钟（用调用者 pid），POSIX 规定
 * 是每进程的——单线程进程下相同；setitimer 的周期语义与三种计时器未实现，如实缺。 */
unsigned int alarm(unsigned int seconds);

/* ttyname：返回该 fd 对应终端的路径（POSIX）。
 *
 * **本系统按设计没有 TTY** ⇒ 本项属「设计性不支持」，不是「缺个组件」。
 * ADR-042（ACCEPTED）明定「canonical 模式与 termios 一概不做」，其事实表记着「全内核无
 * termios/tty/行规程实现」；内核只有输出侧的 console sink（term crate）。
 *
 * 内核里唯一与终端有关的是 INode::is_terminal()——一个节点级布尔真值（ADR-044 / J-TOKEN-A），
 * 唯一用途是让 isatty 说真话。它为真的只有 vfs/src/stdio.rs 的两个节点（fd 0/1/2 背后的
 * **匿名**节点），而那些节点不在任何目录里 ⇒ **没有路径可命名**；
 * 而 /devices/console 没覆写 is_terminal ⇒ 按内核真值是 false，**它不是终端**。
 *
 * 故本实现如实失败（NULL + ENOTTY），不返回伪造路径。将来若真要 ttyname，前提是先有
 * ADR 层面的新裁定（决定引入终端设备节点），而不是在 libc 里补一层转译。 */
char *ttyname(int fd);

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

/* sysconf 的 name 取值（数值与 Linux 一致）。
 *
 * 已实现（真值来源逐条见 libc/src/unistd.rs 的 sysconf 文档）：
 *   _SC_ARG_MAX                 内核装载器的命令行上限（wire.MAX_CMDLINE_BYTES）
 *   _SC_PAGESIZE                内核页大小（values.PAGE_SIZE）
 *   _SC_VERSION / _SC_2_VERSION POSIX.1-2008 = 200809
 *   _SC_NPROCESSORS_CONF / _SC_NPROCESSORS_ONLN  内核 mm::cpu_count()
 * 其余名字如实返回 -1 置 EINVAL（**不编造**返回值；见函数文档的登记）。 */
#define _SC_ARG_MAX 0
#define _SC_VERSION 29
#define _SC_2_VERSION 46
#define _SC_PAGESIZE 30
#define _SC_PAGE_SIZE 30
#define _SC_NPROCESSORS_CONF 83
#define _SC_NPROCESSORS_ONLN 84
#define _SC_CLK_TCK 2
#define _SC_OPEN_MAX 4

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

/* 主机名与登录名（POSIX 归属 unistd.h；实现在 libc/src/posix_batch5.rs）。
 * **诚实边界**：本系统**没有主机名概念**（内核/libsys 均无数据源，已核实）⇒ gethostname 如实
 * 写空串；getlogin 查 LOGNAME/USER，都没有则如实返回 NULL。两者都**不编造**名字。 */
/* exec* 家族（本系统**无法忠实实现**：没有替换进程映像的 syscall）。
 * 提供符号是为了让引用它们的代码能编译/链接（如 libgcc 的 libgcov-interface.c）；
 * **运行时如实返回 -1 并置 ENOTSUP**，绝不假装成功。详见 docs/TODO/libc-posix-surface.md 的 B 类。 */
int execv(const char *path, char *const argv[]);
int execve(const char *path, char *const argv[], char *const envp[]);

int gethostname(char *name, size_t len);
char *getlogin(void);

/* exit/_Exit/abort/atexit 归 <stdlib.h>（POSIX 归属）；unistd.h 只留 _exit。 */
__attribute__((noreturn)) void _exit(int status);

#ifdef __cplusplus
}
#endif


/* ---- 大文件接口（LFS64）别名 ----
 * **Boruix 是 LP64**：`off_t`/`ino_t` 本就是 64 位 ⇒ `*64` 名是**纯别名**（glibc 在 64 位平台同样如此）。
 * 来路：libstdc++ 的 configure 探测 `fseeko64`/`ftello64`/`lseek64`/`stat64`（实测），
 * 缺它们会让 `checking for the value of SEEK_CUR... failed` 而整段 configure 失败。 */
#define lseek64  lseek
#define truncate64 truncate
#define ftruncate64 ftruncate
#define pread64  pread
#define pwrite64 pwrite


/* ---- SEEK_* 与 fseeko/ftello（POSIX）----
 * `SEEK_*` 是 `<stdio.h>`/`<unistd.h>` 的基础宏；`fseeko`/`ftello` 是 `off_t` 版。
 * **Boruix 是 LP64**（`off_t` == `long`）⇒ 二者与 `fseek`/`ftell` 语义等价，故为别名。
 * 来路：libstdc++ 的 configure 检查 `fseeko and ftello`（实测报 'SEEK_CUR' was not declared）。 */
#ifndef SEEK_SET
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#endif
#ifndef fseeko
#define fseeko fseek
#define ftello ftell
#endif

#endif /* _BORUIX_UNISTD_H */
