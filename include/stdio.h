/* stdio.h —— BORUIX libc 标准输入输出。 */
#ifndef _BORUIX_STDIO_H
#define _BORUIX_STDIO_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FILE FILE;

extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

FILE *fopen(const char *path, const char *mode);
int fclose(FILE *fp);
size_t fread(void *ptr, size_t size, size_t nmemb, FILE *fp);
size_t fwrite(const void *ptr, size_t size, size_t nmemb, FILE *fp);
int fflush(FILE *fp);
int fgetc(FILE *fp);
int fputc(int c, FILE *fp);
int ungetc(int c, FILE *fp);
char *fgets(char *s, int n, FILE *fp);
int fputs(const char *s, FILE *fp);
ssize_t getdelim(char **lineptr, size_t *n, int delim, FILE *fp);
ssize_t getline(char **lineptr, size_t *n, FILE *fp);
int feof(FILE *fp);
int ferror(FILE *fp);
int fseek(FILE *fp, long offset, int whence);
long ftell(FILE *fp);

int printf(const char *fmt, ...);
int vprintf(const char *fmt, void *ap);
int fprintf(FILE *fp, const char *fmt, ...);
int vfprintf(FILE *fp, const char *fmt, void *ap);
int sprintf(char *buf, const char *fmt, ...);
int snprintf(char *buf, size_t size, const char *fmt, ...);
int vsnprintf(char *buf, size_t size, const char *fmt, void *ap);
int puts(const char *s);
int putchar(int c);
int getchar(void);
int fscanf(FILE *fp, const char *fmt, ...);

/* libc 初始化（可选，幂等）。 */
void libc_initialize(void);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_STDIO_H */
