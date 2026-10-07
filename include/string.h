/* string.h —— BORUIX libc 字符串与内存函数。 */
#ifndef _BORUIX_STRING_H
#define _BORUIX_STRING_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

void *memcpy(void *dest, const void *src, size_t n);   /* libsys(builtins) */
void *memmove(void *dest, const void *src, size_t n);  /* libsys(builtins) */
void *memset(void *s, int c, size_t n);                /* libsys(builtins) */
int memcmp(const void *a, const void *b, size_t n);    /* libsys(builtins) */
void *memchr(const void *s, int c, size_t n);

size_t strlen(const char *s);
size_t strnlen(const char *s, size_t max);
int strcmp(const char *a, const char *b);
int strncmp(const char *a, const char *b, size_t n);
char *strcpy(char *dst, const char *src);
char *strncpy(char *dst, const char *src, size_t n);
char *strcat(char *dst, const char *src);
char *strchr(const char *s, int c);
char *strrchr(const char *s, int c);
char *strstr(const char *hay, const char *needle);
char *strdup(const char *s);
/* strndup：复制至多 n 字节（POSIX.1-2008）。3P6-2 第二波「整项缺失」类，由
 * libc/tools/audit_posix_surface.py 的反向对账列出。 */
char *strndup(const char *s, size_t n);
size_t strspn(const char *s, const char *accept);
size_t strcspn(const char *s, const char *reject);
char *strpbrk(const char *s, const char *accept);
char *strncat(char *dst, const char *src, size_t n);
char *strtok(char *s, const char *delim);    /* 单线程（内部静态） */
char *strtok_r(char *s, const char *delim, char **saveptr); /* 线程安全 */
int strcasecmp(const char *a, const char *b);
int strncasecmp(const char *a, const char *b, size_t n);
void *memmem(const void *haystack, size_t hl, const void *needle, size_t nl);
char *strsep(char **strp, const char *delim);

/* 错误号 → 人类可读描述（NUL 结尾静态串，调用方不需释放）。 */
char *strerror(int errnum);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_STRING_H */
