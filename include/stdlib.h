/* stdlib.h —— BORUIX libc 标准库杂项。 */
#ifndef _BORUIX_STDLIB_H
#define _BORUIX_STDLIB_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

int abs(int n);
long labs(long n);
long long llabs(long long n);

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
