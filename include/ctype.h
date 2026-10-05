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
int tolower(int c);
int toupper(int c);

#endif /* _CTYPE_H */