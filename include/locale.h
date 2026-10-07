/* locale.h —— BORUIX C 标准库：locale（**诚实最小面**）。
 *
 * 3P6-2 第二波：由 MPC 的真实编译报错驱动补上
 *   get_x.c:28:10: fatal error: 'locale.h' file not found
 *
 * **诚实边界（S09）**：本系统**没有 locale 数据库**（也没有字符集转换表）。故：
 *  - setlocale 只认 "C" / "POSIX"（返回其名），其余**如实返回 NULL**——绝不假装切换成功；
 *  - localeconv 返回 C locale 的固定值（小数点 "."，无千位分隔），这是 C 标准规定的最小行为；
 *  - struct lconv 只声明本系统**有数据来源**的字段（货币等字段无来源，宁缺勿造）。
 */
#ifndef _LOCALE_H
#define _LOCALE_H

#ifdef __cplusplus
extern "C" {
#endif

#define LC_ALL      0
#define LC_COLLATE  1
#define LC_CTYPE    2
#define LC_MONETARY 3
#define LC_NUMERIC  4
#define LC_TIME     5
#define LC_MESSAGES 6

struct lconv {
    char *decimal_point;
    char *thousands_sep;
    char *grouping;
};

char *setlocale(int category, const char *locale);
struct lconv *localeconv(void);

#ifdef __cplusplus
}
#endif

#endif /* _LOCALE_H */
