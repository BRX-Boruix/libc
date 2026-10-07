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
int fstat(int fd, struct stat *buf);
int chmod(const char *path, mode_t mode);
int mkdir(const char *path, mode_t mode);

#endif /* _SYS_STAT_H */