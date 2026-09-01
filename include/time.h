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

time_t time(time_t *tloc);
clock_t clock(void);
int sleep(unsigned seconds);
int usleep(unsigned useconds);
int nanosleep(const struct timespec *req, void *rem);

struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_TIME_H */
