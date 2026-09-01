/* wchar.h —— BORUIX libc 宽字符函数。 */
#ifndef _BORUIX_WCHAR_H
#define _BORUIX_WCHAR_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

size_t wcslen(const wchar_t *s);
int wcscmp(const wchar_t *a, const wchar_t *b);
wchar_t *wcscpy(wchar_t *dst, const wchar_t *src);
wchar_t *wcsncpy(wchar_t *dst, const wchar_t *src, size_t n);
wchar_t *wcscat(wchar_t *dst, const wchar_t *src);
wchar_t *wcschr(const wchar_t *s, wchar_t c);
size_t mbrtowc(wchar_t *pwc, const char *s, size_t n, void *ps);
size_t wcrtomb(char *s, wchar_t wc, void *ps);
size_t mbsrtowcs(wchar_t *dst, const char **src, size_t len, void *ps);
size_t wcsrtombs(char *dst, const wchar_t **src, size_t len, void *ps);
size_t mbstowcs(wchar_t *dst, const char *src, size_t n);
size_t wcstombs(char *dst, const wchar_t *src, size_t n);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_WCHAR_H */
