/* libgen.h —— BORUIX C 标准库：路径分解（POSIX）。
 *
 * 3P6-2 第二波「整项缺失」类：由 libc/tools/audit_posix_surface.py 的反向对账列出。
 *
 * **诚实边界**：
 *  - basename **不改写输入**；无尾部斜杠时返回指向调用方字符串内部的指针，
 *    有尾部斜杠时（结果不是输入的后缀）改返回**静态缓冲**——故该情形下不可重入。
 *    对 "/" 返回 "/"，对空串返回 "."。
 *  - dirname 返回**静态缓冲**中的字符串（POSIX 允许），故不可重入、下次调用会覆盖。
 */
#ifndef _LIBGEN_H
#define _LIBGEN_H

#ifdef __cplusplus
extern "C" {
#endif

char *basename(char *path);
char *dirname(char *path);

#ifdef __cplusplus
}
#endif

#endif /* _LIBGEN_H */
