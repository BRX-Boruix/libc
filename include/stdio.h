/* stdio.h —— BORUIX libc 标准输入输出。 */
#ifndef _BORUIX_STDIO_H
#define _BORUIX_STDIO_H

/* 兼容性：第三方代码常按「stdio 的包含卫哨宏」判断 FILE 是否可用——GMP 的
 * _GMP_H_HAVE_FILE 就检查 _STDIO_H/_STDIO_H_/_STDIO_H_INCLUDED/... 这一串（见 gmp-h.in）。
 * 故除本 libc 自己的 _BORUIX_STDIO_H 外，**再定义**业界惯用的 _STDIO_H。
 *
 * 来路（3P6-2 第二波，真实报错驱动）：交叉构建 GMP 时报
 *   mpz/inp_str.c:63: error: call to undeclared function '__gmpz_inp_str_nowhite'
 * 根因**不是** GMP 缺声明（gmp-impl.h:1784 有），而是它没认出自家的 FILE，
 * 于是那句声明被 #ifdef _GMP_H_HAVE_FILE 挡掉了。 */
#define _STDIO_H 1

#include "boruix_ctypes.h"

/* ---- 标准宏（3P6-3：libstdc++ 的**真实报错**驱动，不是"看起来该有"）----
 *   bits/fstream.tcc:83            error: 'BUFSIZ' was not declared in this scope（19 处）
 *   src/c++98/basic_file.cc:246    error: '_IONBF' was not declared in this scope
 * 取值取 glibc 同值：它们是编译期常量（无 ABI 含义），但保持一致便于对照。 */
#ifndef BUFSIZ
#define BUFSIZ 8192
#endif
#ifndef FOPEN_MAX
#define FOPEN_MAX 16
#endif
#ifndef FILENAME_MAX
#define FILENAME_MAX 4096
#endif
#ifndef L_tmpnam
#define L_tmpnam 20
#endif
#ifndef TMP_MAX
#define TMP_MAX 238328
#endif
#ifndef P_tmpdir
#define P_tmpdir "/tmp"
#endif

/* setvbuf 的 mode 取值（POSIX）。本 libc 的 stdio **不做用户态缓冲**，故三者行为等价；
 * 宏必须存在，否则 libstdc++ 的 basic_file.cc 编不过。 */
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FILE FILE;

extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

FILE *fopen(const char *path, const char *mode);
FILE *fdopen(int fd, const char *mode);
FILE *freopen(const char *path, const char *mode, FILE *fp);
/* 删除文件（或空目录）。成功 0，失败 -1 置 errno。 */
int remove(const char *path);
/* 重命名/移动（POSIX 归属 <stdio.h>）。实现在 libc/src/unistd.rs，此前没有声明
 * （头文件覆盖审计列出）。 */
int rename(const char *oldpath, const char *newpath);
int sscanf(const char *s, const char *fmt, ...);
int fclose(FILE *fp);
size_t fread(void *ptr, size_t size, size_t nmemb, FILE *fp);
size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *fp);
int fflush(FILE *fp);
int fgetc(FILE *fp);
int fputc(int c, FILE *fp);
/* getc/putc：POSIX 里常是宏，本实现提供真函数（取地址/当回调都可用）。
 * 3P6-2 第二波：由交叉构建 GMP 的真实报错驱动补上（mpz/inp_str.c 需要 getc）。 */
int getc(FILE *fp);
int putc(int c, FILE *fp);
int ungetc(int c, FILE *fp);
char *fgets(char *s, int n, FILE *fp);
int fputs(const char *s, FILE *fp);
ssize_t getdelim(char **lineptr, size_t *n, int delim, FILE *fp);
ssize_t getline(char **lineptr, size_t *n, FILE *fp);
int feof(FILE *fp);
int ferror(FILE *fp);
/* 以下 4 项由 3P6-2 第二波的反向对账（libc/tools/audit_posix_surface.py）列出。 */
int fileno(FILE *fp);
void clearerr(FILE *fp);
void rewind(FILE *fp);
void perror(const char *s);
int fseek(FILE *fp, long offset, int whence);
long ftell(FILE *fp);

int printf(const char *fmt, ...);
int vprintf(const char *fmt, void *ap);
int fprintf(FILE *fp, const char *fmt, ...);
int vfprintf(FILE *fp, const char *fmt, void *ap);
int sprintf(char *buf, const char *fmt, ...);
int snprintf(char *buf, size_t size, const char *fmt, ...);
int vsnprintf(char *buf, size_t size, const char *fmt, void *ap);
/* vsprintf：无长度上限的变体（调用方保证缓冲区足够大）。3P6-2 第二波：由交叉构建 GMP 的
 * 真实报错驱动补上（printf/sprintffuns.c 需要它）。 */
int vsprintf(char *buf, const char *fmt, void *ap);
int puts(const char *s);
int putchar(int c);
int getchar(void);
int fscanf(FILE *fp, const char *fmt, ...);

/* scanf：从**标准输入**读格式化输入。与 fscanf/sscanf 共用同一扫描核心 vscan（S15 单点）。 */
int scanf(const char *fmt, ...);

/* tmpfile：创建**自动删除**的临时文件并返回读写流（"w+b" 语义）。
 * 实现：TMPDIR 或 /tmp → mkstemp（O_EXCL 原子独占）→ 立即 unlink → 包成读写流。
 * 目录不存在/不可写或 fd 表满时如实返回 NULL 置 errno（不伪造成功）。 */
FILE *tmpfile(void);

/* libc 初始化（可选，幂等）。 */
void libc_initialize(void);

#ifdef __cplusplus
}
#endif


/* ---- 大文件接口（LFS64）别名 ----
 * **Boruix 是 LP64**：`off_t`/`ino_t` 本就是 64 位 ⇒ `*64` 名是**纯别名**（glibc 在 64 位平台同样如此）。
 * 来路：libstdc++ 的 configure 探测 `fseeko64`/`ftello64`/`lseek64`/`stat64`（实测），
 * 缺它们会让 `checking for the value of SEEK_CUR... failed` 而整段 configure 失败。 */
#define fseeko64 fseek
#define ftello64 ftell
#define fopen64  fopen
#define freopen64 freopen
#define off64_t  off_t
#define fpos64_t fpos_t


/* ---- SEEK_* 与 fseeko/ftello（POSIX）----
 * `SEEK_*` 是 `<stdio.h>`/`<unistd.h>` 的基础宏；`fseeko`/`ftello` 是 `off_t` 版。
 * **Boruix 是 LP64**（`off_t` == `long`）⇒ 二者与 `fseek`/`ftell` 语义等价，故为别名。
 * 来路：libstdc++ 的 configure 检查 `fseeko and ftello`（实测报 'SEEK_CUR' was not declared）。 */
#ifndef SEEK_SET
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#endif
#ifndef fseeko
#define fseeko fseek
#define ftello ftell
#endif

/* fpos_t：POSIX 要求的不透明定位类型。本实现取 `long`（LP64 下与 off_t 同宽），
 * 与 fgetpos/fsetpos 的实现一致（它们委托 ftell/fseek）。 */
typedef long fpos_t;
int fgetpos(FILE *stream, fpos_t *pos);
int fsetpos(FILE *stream, const fpos_t *pos);
/* setbuf/setvbuf：本 libc 的 stdio **不做用户态缓冲** ⇒ 刻意的空操作（见 libc/src/posix_batch6.rs）。 */
void setbuf(FILE *stream, char *buf);
int setvbuf(FILE *stream, char *buf, int mode, size_t size);

#endif /* _BORUIX_STDIO_H */
