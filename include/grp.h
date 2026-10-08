/* grp.h - BORUIX C 标准库：组数据库（3P6-2 第二波，**头文件覆盖审计**驱动）。
 *
 * **为什么现在才有**：getgrnam / getgrgid / endgrent 在 libc/src/pwd.rs 里**早就实现**，
 * 但没有任何头文件声明它们。libc/tools/audit_header_coverage.py 把这个缺口一次列了出来
 * （与 getpid / dup2 / EINTR 同一类：**实现了但没声明**）。
 *
 * **诚实边界（S09）**：
 *  - 本系统**没有 POSIX 的组数据库文件**；组记录来自 /config/groups.json（与 users.json 同源）。
 *    表缺失/不可读 -> 返回 NULL 并置 errno，**绝不返回伪造组**；查不到 -> NULL + ENOENT。
 *  - struct group 的字段顺序必须与 libc/src/pwd.rs 的 #[repr(C)] struct group 逐字段一致。
 *  - gr_mem **恒为 NULL**（本实现不投影成员列表）；要拿某用户的组请用 getgrouplist（见 pwd.h）。
 *  - 只有这三个入口；**没有** setgrent / getgrent —— 本系统不做"遍历组表"的用法，缺就如实缺。
 */
#ifndef _GRP_H
#define _GRP_H

#include "sys/types.h"

struct group {
    char  *gr_name;
    gid_t  gr_gid;
    char **gr_mem;
};

#ifdef __cplusplus
extern "C" {
#endif

struct group *getgrnam(const char *name);
struct group *getgrgid(gid_t gid);
void endgrent(void);

#ifdef __cplusplus
}
#endif

#endif /* _GRP_H */
