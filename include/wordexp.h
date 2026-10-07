/* wordexp.h - BORUIX C 标准库：shell 风格词展开。实现见 libc/src/posix_batch5.rs。
 *
 * **诚实边界**：命令替换（$(...) 与反引号）**不支持** ⇒ 如实返回 WRDE_CMDSUB（POSIX 为此
 * 定义了该错误码），绝不静默展开成空串。未加引号的 shell 元字符（| & ; < > ( ) { }）⇒
 * WRDE_BADCHAR（本系统没有 shell）。`~` 不做家目录展开（无家目录映射约定），按字面保留。
 */
#ifndef _WORDEXP_H
#define _WORDEXP_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    size_t we_wordc;   /* 词数 */
    char **we_wordv;   /* 以 NULL 结尾的词数组 */
    size_t we_offs;    /* WRDE_DOOFFS 时数组前部预留的空槽数 */
} wordexp_t;

#define WRDE_APPEND 1
#define WRDE_DOOFFS 2
#define WRDE_NOCMD  4
#define WRDE_REUSE  8
#define WRDE_SHOWERR 16
#define WRDE_UNDEF  32

#define WRDE_BADCHAR 1
#define WRDE_BADVAL  2
#define WRDE_CMDSUB  3
#define WRDE_NOSPACE 4
#define WRDE_SYNTAX  6

int wordexp(const char *words, wordexp_t *pwordexp, int flags);
void wordfree(wordexp_t *pwordexp);

#ifdef __cplusplus
}
#endif

#endif /* _WORDEXP_H */