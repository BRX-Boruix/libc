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

/* d_type 取值（与 glibc 对齐）。 */
#define DT_UNKNOWN 0
#define DT_DIR     4
#define DT_REG     8

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
int rewinddir(DIR *dirp);

#endif /* _DIRENT_H */