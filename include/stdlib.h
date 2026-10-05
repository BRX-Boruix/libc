/* stdlib.h —— BORUIX libc 标准库杂项。 */
#ifndef _BORUIX_STDLIB_H
#define _BORUIX_STDLIB_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/* 共享库 ABI 版本探针（阶段 5 / 3P5-2）。**新增导出**，不影响既有程序。 */
int boruix_so_abi_version(void);

int abs(int n);
long labs(long n);
long long llabs(long long n);

/* 进程终止与退出处理（POSIX 把这三个放在 stdlib.h）。 */
__attribute__((noreturn)) void exit(int status);
__attribute__((noreturn)) void _Exit(int status);
__attribute__((noreturn)) void abort(void);
int atexit(void (*func)(void));

/* 动态内存（另见 malloc.h；POSIX 同样在 stdlib.h 声明这四个）。 */
void *malloc(size_t size);
void *calloc(size_t nmemb, size_t size);
void *realloc(void *ptr, size_t size);
void free(void *ptr);

int atoi(const char *s);
long atol(const char *s);
long long atoll(const char *s);

long strtol(const char *s, char **endptr, int base);
unsigned long strtoul(const char *s, char **endptr, int base);
long long strtoll(const char *s, char **endptr, int base);
unsigned long long strtoull(const char *s, char **endptr, int base);

double strtod(const char *s, char **endptr);
float strtof(const char *s, char **endptr);
long double strtold(const char *s, char **endptr);

int rand(void);
void srand(unsigned seed);

typedef struct { int quot; int rem; } div_t;
typedef struct { long quot; long rem; } ldiv_t;
div_t div(int numer, int denom);
ldiv_t ldiv(long numer, long denom);

typedef int (*__compare_fn_t)(const void *, const void *);
void qsort(void *base, size_t nmemb, size_t size, __compare_fn_t cmp);
void *bsearch(const void *key, const void *base, size_t nmemb, size_t size, __compare_fn_t cmp);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_STDLIB_H */
