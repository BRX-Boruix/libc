/* fnmatch.h - BORUIX C 标准库：文件名模式匹配（POSIX）。
 * 实现见 libc/src/posix_batch4.rs。
 *
 * **诚实边界**：GNU 扩展 FNM_CASEFOLD / FNM_LEADING_DIR / FNM_EXPORT 均不支持——
 * 传入未定义的标志位会**如实**返回 FNM_NOMATCH 并置 EINVAL，绝不静默忽略。
 */
#ifndef _FNMATCH_H
#define _FNMATCH_H

#define FNM_NOMATCH  1
#define FNM_PATHNAME (1 << 0)
#define FNM_NOESCAPE (1 << 1)
#define FNM_PERIOD   (1 << 2)

#ifdef __cplusplus
extern "C" {
#endif

int fnmatch(const char *pattern, const char *string, int flags);

#ifdef __cplusplus
}
#endif

#endif /* _FNMATCH_H */