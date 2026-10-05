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

time_t time(time_t *tloc);
clock_t clock(void);
int sleep(unsigned seconds);
int usleep(unsigned useconds);
int nanosleep(const struct timespec *req, void *rem);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_TIME_H */
