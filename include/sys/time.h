/* sys/time.h —— BORUIX libc：BSD 风格时间接口（阶段 6 / 3P6-2）。
 *
 * **诚实边界（S39）**：本系统的墙钟来自内核 RTC 直读 CMOS，**只有秒级分辨率**。
 * 因此 `gettimeofday` 的 `tv_usec` **恒为 0**——我们不去用单调时钟的亚秒部分凑一个
 * 「看起来更精确」的微秒值（那是把两个不同时间源拼在一起，属于伪造数据，S09）。
 * 需要亚秒级**间隔**测量的程序应使用 `clock()`（单调，纳秒刻度，实际分辨率见其说明）。
 */
#ifndef _BORUIX_SYS_TIME_H
#define _BORUIX_SYS_TIME_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/* time_t 与 time.h 保持同一类型（单点定义见 libc/src/time.rs）。 */
#ifndef _BORUIX_TIME_T_DEFINED
#define _BORUIX_TIME_T_DEFINED
typedef long time_t;
#endif

struct timeval {
    time_t tv_sec;   /* 自 Unix epoch 的秒数 */
    long   tv_usec;  /* 微秒；**本系统恒为 0**（见文件头诚实边界） */
};

/* 历史遗留参数：POSIX 已废弃，仅为兼容既有源码保留。 */
struct timezone {
    int tz_minuteswest;
    int tz_dsttime;
};

/* 成功返回 0；墙钟不可用时返回 -1 并置 errno（与 time() 同一条数据链路）。 */
int gettimeofday(struct timeval *tv, void *tz);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_SYS_TIME_H */