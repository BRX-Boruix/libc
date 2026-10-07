/* ctype.h - BORUIX C 标准库：字符分类与转换（3P3-2）。
 *
 * 实现见 libc/src/ctype.rs（C ABI 导出）。语义遵循 C 标准：入参是 int，可表示
 * EOF(-1)；不可表示或非本类的字符返回 0。
 */
#ifndef _CTYPE_H
#define _CTYPE_H

int isalpha(int c);
int isdigit(int c);
int isalnum(int c);
int isupper(int c);
int islower(int c);
int isspace(int c);
int isxdigit(int c);
int isprint(int c);
int ispunct(int c);
int iscntrl(int c);
int isgraph(int c);
int isblank(int c);
/* isascii：POSIX.1-2008 已移出标准，但现实代码大量使用。glibc 同型：**宏优先**，
 * 真函数也存在（写 (isascii)(c) 或取地址时用得到）。
 * 3P6-2 第二波：由交叉构建 GMP 的真实报错驱动补上（printf/doprnt.c 需要它）。 */
int isascii(int c);
#define isascii(c) (((c) & ~0x7F) == 0)
int tolower(int c);
int toupper(int c);

#endif /* _CTYPE_H */