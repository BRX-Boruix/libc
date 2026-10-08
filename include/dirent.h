/* dirent.h - BORUIX C 标准库：目录流（3P3-2）。
 *
 * `struct dirent` 的布局**必须与 Rust 侧 libc/src/dirent.rs 的 #[repr(C)] 逐字段一致**
 * （双方都是 repr(C) + 同字段同序，故自动一致；驱动里有 _Static_assert 锚定）。
 *
 * 诚实边界：`d_ino` 恒 0（内核目录项不含 inode 号，不伪造）；`d_type` 由内核 type 标签映射。
 */
#ifndef _DIRENT_H
#define _DIRENT_H

#include "sys/types.h"

#ifdef __cplusplus
extern "C" {
#endif

/* d_type 取值（与 glibc 对齐）。3P6-3：libstdc++ 用到 DT_LNK/DT_SOCK/DT_FIFO/DT_CHR/DT_BLK，
 * 只定义 UNKNOWN/DIR/REG 会报 'DT_SOCK' was not declared。这里补成**完整一套**（一次补完，
 * 不留"下次再缺一个"的口子）。 */
#define DT_UNKNOWN 0
#define DT_FIFO    1
#define DT_CHR     2
#define DT_DIR     4
#define DT_BLK     6
#define DT_REG     8
#define DT_LNK     10
#define DT_SOCK    12
#define DT_WHT     14

struct dirent {
    ino_t          d_ino;
    off_t          d_off;
    unsigned short d_reclen;
    unsigned char  d_type;
    char           d_name[256];
};

typedef struct DIR DIR; /* 不透明：内部结构属实现细节 */

DIR *opendir(const char *path);
struct dirent *readdir(DIR *dirp);
int closedir(DIR *dirp);
/* rewinddir 按 POSIX 返回 void（此前这里误声明为 int——与 POSIX 不符，已改正）。 */
void rewinddir(DIR *dirp);

/* telldir/seekdir：目录流位置。本系统的目录流是**打开时的快照**，故位置就是项序号；
 * telldir 的返回值对调用方不透明，只保证可传给同一流的 seekdir（POSIX 契约）。
 * 3P6-2 第二波 C2 核实：内核/libsys 不需要新能力，纯粹是本层实现——已落地。 */
long telldir(DIR *dirp);
void seekdir(DIR *dirp, long loc);

/* scandir：把目录项读成排序后的数组（元素与数组都由 malloc 分配，调用方逐个 free）。
 * compar 常用 alphasort（本实现用 strcmp：本系统只有 C locale，POSIX 的 strcoll 已判不支持）。 */
int scandir(const char *dirp,
            struct dirent ***namelist,
            int (*filter)(const struct dirent *),
            int (*compar)(const struct dirent **, const struct dirent **));
int alphasort(const struct dirent **a, const struct dirent **b);

#ifdef __cplusplus
}
#endif

#endif /* _DIRENT_H */