/* pwd.h - BORUIX C 标准库：用户数据库（3P3-2）。
 *
 * **注意：`struct passwd` 的字段顺序与 POSIX/glibc 不同**——本系统的 Rust 侧
 * （libc/src/pwd.rs）只保留 name/uid/gid/dir/shell 五项且按此顺序布局；头文件必须照它写，
 * 否则跨边界读到的就是错位数据。这是**有意精简**，不是遗漏（POSIX 的 pw_passwd/pw_gecos
 * 在本系统无对应来源，宁缺勿造）。
 */
#ifndef _PWD_H
#define _PWD_H

#include "sys/types.h"

struct passwd {
    char  *pw_name;
    uid_t  pw_uid;
    gid_t  pw_gid;
    char  *pw_dir;
    char  *pw_shell;
};

#ifdef __cplusplus
extern "C" {
#endif

struct passwd *getpwnam(const char *name);
struct passwd *getpwuid(uid_t uid);
void setpwent(void);
void endpwent(void);
struct passwd *getpwent(void);

#ifdef __cplusplus
}
#endif

#endif /* _PWD_H */