/* math.h - BORUIX C 标准库：数学函数（3P3-2）。
 *
 * 实现见 libc/src/math_core.rs + libc/src/math_core2.rs（纯计算核心）与
 * libc/src/math_exports.rs（C ABI 导出）。**函数体在 libc.a 内**——本系统没有
 * 单独的 libm，因此链接时不需要 -lm。
 *
 * 精度（B 档：≤1 ulp）由**宿主对照验证器**实测：
 *   tools/checks/math_verify/verify.rs（5231 个样本 + 边界值，与宿主 glibc libm 逐点比 ULP）
 * 当前 27/27 项 ≤1 ulp，其中 sqrt / log2 / asin 为 0 ulp。
 *
 * 诚实边界（不假装支持）：
 *   - 三角函数用 fdlibm 三轮 Cody-Waite 归约，**有效范围 |x| < 2^20**；
 *     超出需 Payne-Hanek（未实现），此时不作 ≤1 ulp 承诺。
 *   - f32 版是"f64 核心 + 一次收窄"（见 math_exports.rs 的说明）。
 *   - 无 long double 版数学函数（仅 ldexpl，见 longdouble 实现）。
 */
#ifndef _MATH_H
#define _MATH_H

#define HUGE_VAL (__builtin_huge_val())
#define HUGE_VALF (__builtin_huge_valf())
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))

/* ---- double：取整 / 分解 ---- */
double fabs(double x);
double copysign(double x, double y);
double floor(double x);
double ceil(double x);
double trunc(double x);
double round(double x);
double nearbyint(double x);
double nextafter(double x, double y);
double modf(double x, double *iptr);
double frexp(double x, int *exp);
double ldexp(double x, int exp);

/* ---- double：幂 / 根 ---- */
double sqrt(double x);
double cbrt(double x);
double hypot(double x, double y);
double pow(double x, double y);
double fmod(double x, double y);

/* ---- double：指数 / 对数 ---- */
double exp(double x);
double expm1(double x);
double log(double x);
double log1p(double x);
double log2(double x);
double log10(double x);

/* ---- double：三角 / 双曲 ---- */
double sin(double x);
double cos(double x);
double tan(double x);
double asin(double x);
double acos(double x);
double atan(double x);
double atan2(double y, double x);
double sinh(double x);
double cosh(double x);
double tanh(double x);

/* ---- float ---- */
float fabsf(float x);
float copysignf(float x, float y);
float floorf(float x);
float ceilf(float x);
float truncf(float x);
float roundf(float x);
float nearbyintf(float x);
float nextafterf(float x, float y);
float modff(float x, float *iptr);
float frexpf(float x, int *exp);
float ldexpf(float x, int exp);

float sqrtf(float x);
float cbrtf(float x);
float hypotf(float x, float y);
float powf(float x, float y);
float fmodf(float x, float y);

float expf(float x);
float expm1f(float x);
float logf(float x);
float log1pf(float x);
float log2f(float x);
float log10f(float x);

float sinf(float x);
float cosf(float x);
float tanf(float x);
float asinf(float x);
float acosf(float x);
float atanf(float x);
float atan2f(float y, float x);
float sinhf(float x);
float coshf(float x);
float tanhf(float x);

/* long double 版：本目标 long double 是 x87 80 位（实测 16 字节），实现见 libc/src/longdouble.rs。 */
long double ldexpl(long double x, int exp);

#endif /* _MATH_H */
