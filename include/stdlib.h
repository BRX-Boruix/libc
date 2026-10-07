/* stdlib.h —— BORUIX libc 标准库杂项。 */
#ifndef _BORUIX_STDLIB_H
#define _BORUIX_STDLIB_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

/* 共享库 ABI 版本探针（阶段 5 / 3P5-2）。**新增导出**，不影响既有程序。 */
int boruix_so_abi_version(void);

/* 环境访问（3P6-2）。`environ` 由 C 入口桥接（csrc/user_main.c）在调用 main 之前注册；
 * 不经该桥接的程序（如 Rust 程序）其 `environ` 保持 NULL——**已声明的边界**。 */
extern char **environ;
char *getenv(const char *name);

/* 路径规范化（POSIX）。`resolved == NULL` 时由本函数 malloc，**调用方负责 free**。 */
char *realpath(const char *path, char *resolved);

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
/* atof：等价 strtod(s, NULL)。3P6-2 第二波「整项缺失」类，反向对账（audit_posix_surface.py）列出。 */
double atof(const char *s);
/* mktemp：POSIX 已标记为不安全（有竞态）；本实现只做最小语义，不声称原子。
 * 3P6-2 第二波：由 GCC 宿主侧构建的真实报错驱动补上。 */
char *mktemp(char *template);
/* random/srandom：与 rand/srand 同一生成器（POSIX 要求两者是同一序列的两个接口）。 */
long random(void);
void srandom(unsigned int seed);

/* 环境表修改（3P6-2 第二波「整项缺失」类，反向对账列出）。
 * 诚实边界：环境表是进程全局的，本实现不加锁——并发 setenv/putenv 与 getenv 需调用方自行同步。
 * putenv 不复制字符串（POSIX：该字符串成为环境的一部分，调用方不得释放）。 */
int setenv(const char *name, const char *value, int overwrite);
int unsetenv(const char *name);
int putenv(char *string);
int clearenv(void);
float strtof(const char *s, char **endptr);
long double strtold(const char *s, char **endptr);

int rand(void);
void srand(unsigned seed);

/* mkstemp / mkdtemp：**原子独占**创建唯一临时文件/目录（POSIX）。
 * 模板末尾必须是 6 个 'X'，成功时就地替换。
 * 原子性来自内核 O_EXCL（open 标志 bit 8，3P6-2 第二波新增）：内核在同一 syscall 内
 * 完成「存在性判定 + 创建」，没有 TOCTOU 窗口。此前无此位，这两项只能「先 stat 再
 * create」，两个进程会互相覆盖临时文件——故 docs/TODO/libc-posix-surface.md 曾把它们
 * 登记为「判定不支持」；内核位落地后该判定**已撤回**。 */
int mkstemp(char *template);
char *mkdtemp(char *template);

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
