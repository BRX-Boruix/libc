/* math.h - BORUIX C 标准库：数学函数（3P3-2）。
 *
 * 实现见 libc/src/float.rs，**函数体在 libc.a 内**——本系统没有单独的 libm，
 * 因此链接时不需要 -lm。
 */
#ifndef _MATH_H
#define _MATH_H

#define HUGE_VAL (__builtin_huge_val())
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))

double sqrt(double x);
float sqrtf(float x);
double floor(double x);
float floorf(float x);
double ceil(double x);
float ceilf(float x);
double pow(double x, double y);
float powf(float x, float y);
double fabs(double x);
float fabsf(float x);
double sin(double x);
double cos(double x);
double tan(double x);
double log(double x);
double log2(double x);
double log10(double x);
double exp(double x);
double round(double x);
double trunc(double x);
double fmod(double x, double y);
/* frexp/ldexp：浮点分解与重组（3P6-2 第二波，由 MPC 的真实报错
 * radius.c: call to undeclared function 'frexp' 驱动补上）。
 * frexp 把 x 写成 m * 2^e 且 |m| ∈ [0.5,1)；ldexp 是它的逆。 */
double frexp(double x, int *exp);
double ldexp(double x, int exp);
/* long double 版：本目标 long double 是 x87 80 位（实测 16 字节），实现见 libc/src/float.rs。 */
long double ldexpl(long double x, int exp);

#endif /* _MATH_H */