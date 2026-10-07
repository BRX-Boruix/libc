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
/* GNU 扩展（取值与 glibc 一致）。**来路**：GCC 自带的 libiberty/fnmatch.c 直接使用它们，
 * 交叉构建时报 `use of undeclared identifier 'FNM_CASEFOLD'`（真实报错驱动，不是预猜）。 */
#define FNM_LEADING_DIR (1 << 3)   /* 匹配到某个 '/' 边界即算命中 */
#define FNM_CASEFOLD    (1 << 4)   /* 大小写不敏感（本实现只折 ASCII：无 locale 数据） */
#define FNM_FILE_NAME   FNM_PATHNAME  /* glibc 的别名，同值同义 */

#ifdef __cplusplus
extern "C" {
#endif

int fnmatch(const char *pattern, const char *string, int flags);

#ifdef __cplusplus
}
#endif

#endif /* _FNMATCH_H */