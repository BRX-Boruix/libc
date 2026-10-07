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

#ifdef __cplusplus
}
#endif

#endif /* _GETOPT_H */