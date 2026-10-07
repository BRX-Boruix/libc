/* time.h —— BORUIX libc 时间函数。 */
#ifndef _BORUIX_TIME_H
#define _BORUIX_TIME_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef long time_t;
typedef long clock_t;
#define CLOCKS_PER_SEC 1000000000L

/* struct timespec 必须在任何使用它的原型**之前**定义：否则 `const struct timespec *`
 * 会被当作函数内新声明的不可见类型（实测警告 -Wvisibility），调用方拿到的就是另一个类型。 */
struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

/* 日历时间。字段顺序必须与 Rust 侧 libc/src/time.rs 的 #[repr(C)] struct Tm 一致。 */
struct tm {
    int tm_sec;    /* 秒 0-60 */
    int tm_min;    /* 分 0-59 */
    int tm_hour;   /* 时 0-23 */
    int tm_mday;   /* 日 1-31 */
    int tm_mon;    /* 月 0-11（注意：**0 起**，与直觉差 1） */
    int tm_year;   /* 自 1900 起的年数（注意：**减 1900**） */
    int tm_wday;   /* 星期 0-6，0=周日 */
    int tm_yday;   /* 年内第几天 0-365 */
    int tm_isdst;  /* 夏令时标志；本系统恒为 0 */
};

/* 两者都返回指向**静态存储**的指针（后续调用会覆盖，POSIX 允许）。
 * **诚实边界**：本系统无时区数据库，故 localtime 与 gmtime 行为完全相同（按 UTC）。 */
struct tm *gmtime(const time_t *t);
struct tm *localtime(const time_t *t);

time_t time(time_t *tloc);
clock_t clock(void);

/* strftime：按格式串把 tm 渲染进 s（最多 max 字节，**含**结尾 NUL 的位置）。
 * 返回写入的字节数（不含 NUL）；放不下或参数非法返回 0。
 *
 * 支持集与"未实现项"的显式清单见 libc/src/time.rs 的 strftime 文档（S09 不夸大）。 */
size_t strftime(char *s, size_t max, const char *format, const struct tm *tm);
/* difftime：两时刻之差（秒，double）。3P6-2 第二波「整项缺失」类，反向对账列出。 */
double difftime(time_t t1, time_t t0);
int sleep(unsigned seconds);
int usleep(unsigned useconds);
int nanosleep(const struct timespec *req, void *rem);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_TIME_H */
