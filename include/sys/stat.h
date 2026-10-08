/* sys/stat.h - BORUIX C 标准库：文件状态（3P3-2）。
 *
 * `struct stat` 是**诚实子集**（见 libc/src/unistd.rs 的说明）：内核只暴露 r/w/x +
 * system_only 四个布尔，**不存在** owner/group/other 权限矩阵。故：
 *   - st_mode 的类型位（S_IF*）如实填入；
 *   - 权限位：owner rwx 为真值，组/其他位**镜像 owner**（单用户擦平）；
 *   - st_ino / st_dev 等内核未暴露的字段**如实置 0，不伪造**。
 *
 * 布局必须与 Rust 侧 libc/src/unistd.rs 的 #[repr(C)] struct stat 逐字段一致；
 * 驱动里有 _Static_assert 锚定。
 */
#ifndef _SYS_STAT_H
#define _SYS_STAT_H

#include "types.h"
#include "../time.h"

struct stat {
    unsigned long   st_dev;
    unsigned long   st_ino;
    unsigned long   st_nlink;
    unsigned int    st_mode;
    unsigned int    st_uid;
    unsigned int    st_gid;
    int             __pad0;
    unsigned long   st_rdev;
    long            st_size;
    long            st_blksize;
    long            st_blocks;
    struct timespec st_atim;
    struct timespec st_mtim;
    struct timespec st_ctim;
    long            __unused[3];
};

/* st_mode 的类型位（与 libc/src/unistd.rs 的 S_IF* 常量同一事实）。 */
#define S_IFMT   0170000
#define S_IFSOCK 0140000
#define S_IFLNK  0120000
#define S_IFREG  0100000
#define S_IFBLK  0060000
#define S_IFDIR  0040000
#define S_IFCHR  0020000
#define S_IFIFO  0010000

#define S_ISDIR(m)  (((m) & S_IFMT) == S_IFDIR)
#define S_ISREG(m)  (((m) & S_IFMT) == S_IFREG)
#define S_ISLNK(m)  (((m) & S_IFMT) == S_IFLNK)
#define S_ISCHR(m)  (((m) & S_IFMT) == S_IFCHR)
#define S_ISBLK(m)  (((m) & S_IFMT) == S_IFBLK)
#define S_ISFIFO(m) (((m) & S_IFMT) == S_IFIFO)
#define S_ISSOCK(m) (((m) & S_IFMT) == S_IFSOCK)

/* POSIX/glibc 的便捷访问宏：st_atime/st_mtime/st_ctime 就是对应 timespec 的 tv_sec。
 *
 * 来路（3P6-2 第二波，真实报错驱动）：新写的 ls 工具（tools/3psrc/bxls）在系统内用 tcc
 * 编译时报 `bxls.c:76: error: field not found: st_mtime`——本头文件只暴露了 `st_mtim`，
 * 而真实程序写的是 POSIX 名字。补上同名宏，**不另造字段**（语义与 glibc 完全一致）。 */
#define st_atime st_atim.tv_sec
#define st_mtime st_mtim.tv_sec
#define st_ctime st_ctim.tv_sec

int stat(const char *path, struct stat *buf);
/* lstat：不跟随符号链接的 stat。**诚实边界**：本系统无 no-follow 原语，故实现为
 * 「先 readlink 判链接：是链接则合成 S_IFLNK + st_size=目标长度；否则退回 stat」。
 * 3P6-2 第二波：由 GCC 宿主侧构建的真实报错驱动补上。 */
int lstat(const char *path, struct stat *buf);
int fstat(int fd, struct stat *buf);
int chmod(const char *path, mode_t mode);
int mkdir(const char *path, mode_t mode);


/* ---- 大文件接口（LFS64）别名 ----
 * **Boruix 是 LP64**：`off_t`/`ino_t` 本就是 64 位 ⇒ `*64` 名是**纯别名**（glibc 在 64 位平台同样如此）。
 * 来路：libstdc++ 的 configure 探测 `fseeko64`/`ftello64`/`lseek64`/`stat64`（实测），
 * 缺它们会让 `checking for the value of SEEK_CUR... failed` 而整段 configure 失败。 */
#define stat64   stat
#define fstat64  fstat
#define lstat64  lstat

#endif /* _SYS_STAT_H */