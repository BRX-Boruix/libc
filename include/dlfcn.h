/* dlfcn.h —— 动态装载（POSIX）。
 *
 * **为什么此前不存在**：机内实测 `tcc` 报 `include file 'dlfcn.h' not found` ⇒ C 程序
 * 连 `dlopen`/`dlsym`/`RTLD_*` 的**声明**都拿不到，只能自己手写 `extern`，而这正是
 * 本项（清单第 6 项）卡住验证的那一步。
 *
 * 取值与 glibc 一致（编译期常量，无 ABI 含义，保持一致便于对照）。 */
#ifndef _BORUIX_DLFCN_H
#define _BORUIX_DLFCN_H

#ifdef __cplusplus
extern "C" {
#endif

/* flags。**本实现的实际支持面见 rtld/src/main.rs 的 dlopen 文档**：
 * RTLD_LAZY 与 RTLD_NOW 无区别（一律立即解析全部重定位）；
 * RTLD_GLOBAL/RTLD_LOCAL 语义支持；RTLD_NOLOAD 见该处说明。 */
#define RTLD_LAZY   0x00001
#define RTLD_NOW    0x00002
#define RTLD_NOLOAD 0x00004
#define RTLD_GLOBAL 0x00100
#define RTLD_LOCAL  0x00000

/* 失败返回 NULL（`dlopen`/`dlsym`）。`dlclose` 成功 0、句柄非法 -1。 */
void *dlopen(const char *path, int flags);
void *dlsym(void *handle, const char *name);
int   dlclose(void *handle);
/* 最近一次错误串；无错返回 NULL。 */
char *dlerror(void);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_DLFCN_H */