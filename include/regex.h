/* regex.h - BORUIX C 标准库：POSIX 正则。实现见 libc/src/regex.rs。
 *
 * **本布局是 BORUIX 自有 ABI，不与 glibc 的 regex_t 二进制兼容**——跨 libc 传 regex_t
 * 本来就不可移植（POSIX 只说它是结构类型）。re_nsub 按 POSIX 要求可读。
 *
 * **诚实边界**：反向引用（\1..\9）不支持（REG_ESUBREG）；字符类 [[:alpha:]] 不支持
 * （REG_ECTYPE，本系统无 locale 表）；{m,n} 的 n 超过 255 报 REG_BADBR（不静默截断）。
 */
#ifndef _REGEX_H
#define _REGEX_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef long regoff_t;

typedef struct {
    void  *prog;     /* 内部：编译后的指令数组 */
    size_t re_nsub;  /* 捕获组数（不含整体匹配的第 0 组） */
    int    cflags;
    int    errcode;
} regex_t;

typedef struct {
    regoff_t rm_so;  /* 匹配起始偏移；未参与匹配的组为 -1 */
    regoff_t rm_eo;  /* 匹配结束偏移（不含） */
} regmatch_t;

#define REG_EXTENDED 1
#define REG_ICASE    2
#define REG_NEWLINE  4
#define REG_NOSUB    8

#define REG_NOTBOL 1
#define REG_NOTEOL 2

#define REG_NOMATCH  1
#define REG_BADPAT   2
#define REG_ECOLLATE 3
#define REG_ECTYPE   4
#define REG_EESCAPE  5
#define REG_ESUBREG  6
#define REG_EBRACK   7
#define REG_EPAREN   8
#define REG_EBRACE   9
#define REG_BADBR    10
#define REG_ERANGE   11
#define REG_ESPACE   12
#define REG_BADRPT   13

int regcomp(regex_t *preg, const char *regex, int cflags);
int regexec(const regex_t *preg, const char *string, size_t nmatch,
            regmatch_t pmatch[], int eflags);
size_t regerror(int errcode, const regex_t *preg, char *errbuf, size_t errbuf_size);
void regfree(regex_t *preg);

#ifdef __cplusplus
}
#endif

#endif /* _REGEX_H */