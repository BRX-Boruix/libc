/* getopt.h - BORUIX C 标准库：命令行选项解析。实现见 libc/src/posix_batch4.rs。
 *
 * getopt 的 POSIX 归属是 <unistd.h>，故它也在那里声明（两处同一原型，不冲突）。
 *
 * **诚实边界**：getopt **不做参数置换**（GNU 的「把非选项挪到末尾」）——遇到第一个非选项
 * 即停止。这是 POSIX 允许的经典行为；需要置换的调用方请显式排序或改用 getopt_long。
 */
#ifndef _GETOPT_H
#define _GETOPT_H

#ifdef __cplusplus
extern "C" {
#endif

struct option {
    const char *name;
    int         has_arg;
    int        *flag;
    int         val;
};

#define no_argument       0
#define required_argument 1
#define optional_argument 2

int getopt(int argc, char *const argv[], const char *optstring);
int getopt_long(int argc, char *const argv[], const char *optstring,
                const struct option *longopts, int *longindex);

/* _getopt_internal：**glibc 的内部入口**，由 libiberty/getopt1.c 直接调用
 * （交叉构建实测报 `call to undeclared function '_getopt_internal'`）。
 * 本实现把它作为 getopt/getopt_long 的**共同核心**（S15 单点）：longopts==NULL ⇒ 短选项语义。
 * **诚实边界**：long_only != 0（把 -xyz 也当长选项试）是 GNU 扩展，本实现**如实返回 ENOTSUP**，
 * 不静默按 0 处理——libiberty 传的正是 0。 */
int _getopt_internal(int argc, char *const argv[], const char *optstring,
                     const struct option *longopts, int *longind, int long_only);

#ifdef __cplusplus
}
#endif

#endif /* _GETOPT_H */