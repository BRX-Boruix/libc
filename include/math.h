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

#endif /* _MATH_H */