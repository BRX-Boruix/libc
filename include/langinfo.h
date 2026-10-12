/* langinfo.h —— BORUIX libc 的本地化信息（POSIX nl_langinfo）。
 *
 * 本系统只有 C locale（无 locale 数据库，setlocale 已判不支持），故 nl_langinfo 返回
 * C locale 的规范值。**编号与 FreeBSD/POSIX 的枚举一致**（逐项从
 * envfiles/msys2/.../include/langinfo.h 解析得到，不靠记忆）。
 *
 * 诚实边界：ERA 族与 ALT_DIGITS(45..49)、THOUSEP(51)、YESSTR 与 NOSTR(54/55)、
 * CRNCYSTR(56) 在 C locale 下确实无值，返回空串——那是 POSIX 的表示，不是缺实现。
 */
#ifndef _BORUIX_LANGINFO_H
#define _BORUIX_LANGINFO_H

#include "boruix_ctypes.h"

typedef int nl_item;

enum {
    CODESET = 0,
    D_T_FMT = 1,
    D_FMT = 2,
    T_FMT = 3,
    T_FMT_AMPM = 4,
    AM_STR = 5,
    PM_STR = 6,
    DAY_1 = 7, DAY_2, DAY_3, DAY_4, DAY_5, DAY_6, DAY_7,
    ABDAY_1 = 14, ABDAY_2, ABDAY_3, ABDAY_4, ABDAY_5, ABDAY_6, ABDAY_7,
    MON_1 = 21, MON_2, MON_3, MON_4, MON_5, MON_6, MON_7,
    MON_8, MON_9, MON_10, MON_11, MON_12,
    ABMON_1 = 33, ABMON_2, ABMON_3, ABMON_4, ABMON_5, ABMON_6,
    ABMON_7, ABMON_8, ABMON_9, ABMON_10, ABMON_11, ABMON_12,
    ERA = 45, ERA_D_FMT, ERA_D_T_FMT, ERA_T_FMT, ALT_DIGITS,
    RADIXCHAR = 50,
    THOUSEP = 51,
    YESEXPR = 52,
    NOEXPR = 53,
    YESSTR = 54,
    NOSTR = 55,
    CRNCYSTR = 56
};

#ifdef __cplusplus
extern "C" {
#endif

char *nl_langinfo(nl_item item);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_LANGINFO_H */
