/* glob.h - BORUIX C 标准库：路径名模式展开（POSIX）。实现见 libc/src/glob.rs。
 *
 * **诚实边界**：GLOB_BRACE / GLOB_TILDE / GLOB_ONLYDIR / GLOB_ALTDIRFUNC 不支持——
 * 传入即返回 GLOB_ABORTED 并置 ENOTSUP（绝不静默忽略，否则调用方会以为花括号展开了）。
 *
 * gl_pathv 里的字符串与数组都由 malloc 分配，由 globfree 释放。
 */
#ifndef _GLOB_H
#define _GLOB_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

struct stat;

typedef struct {
    size_t gl_pathc;    /* 匹配到的路径数（不含结尾 NULL） */
    char **gl_pathv;    /* 以 NULL 结尾的路径数组 */
    size_t gl_offs;     /* GLOB_DOOFFS 时数组前部预留的空槽数 */
} glob_t;

#define GLOB_ERR      (1 << 0)
#define GLOB_MARK     (1 << 1)
#define GLOB_NOSORT   (1 << 2)
#define GLOB_DOOFFS   (1 << 3)
#define GLOB_NOCHECK  (1 << 4)
#define GLOB_APPEND   (1 << 5)
#define GLOB_NOESCAPE (1 << 6)
#define GLOB_PERIOD   (1 << 7)

#define GLOB_NOSPACE 1
#define GLOB_ABORTED 2
#define GLOB_NOMATCH 3

int glob(const char *pattern, int flags,
         int (*errfunc)(const char *epath, int eerrno), glob_t *pglob);
void globfree(glob_t *pglob);

#ifdef __cplusplus
}
#endif

#endif /* _GLOB_H */