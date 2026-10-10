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

/* extern "C"：这些都是 **C 符号**。缺了这层保护，C++ 程序（含 libstdc++）看到的是
 * C++ 名字修饰的声明 ⇒ 一是链接期找不到符号，二是与 libstdc++ 自己的 `extern "C"`
 * 定义**冲突**。3P6-3 实测：`math_stubs_float.cc` 报
 * `conflicting declaration of 'float fabsf(float)' with 'C' linkage`，一次 60+ 条。 */
#ifdef __cplusplus
extern "C" {
#endif

#define HUGE_VAL (__builtin_huge_val())
#define HUGE_VALF (__builtin_huge_valf())
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))

/* 浮点分类常量（C99）。3P6-3：libstdc++ 的 src/c++17/floating_to_chars.cc 用
 * __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, x)，
 * 缺一个就报 'FP_NAN' was not declared。取值取 glibc 同值。 */
#define FP_NAN       0
#define FP_INFINITE  1
#define FP_ZERO      2
#define FP_SUBNORMAL 3
#define FP_NORMAL    4

/* ---- C99 浮点分类（7.12.3）----
 *
 * **此前只有上面的 FP_* 常量、没有这些宏，也没有同名函数**（3P6-3 记录的真实阻塞）。
 * 两层后果：① C 程序用不了 C99 分类；② C++ 的 `std::isnan` 更用不了——libstdc++ 的
 * `<cmath>` 会 `#undef` 掉宏、再用 `using ::isnan;` 引入**函数**，故 `::isnan` 必须真实存在。
 *
 * **顺序要紧（实测教训）**：函数声明必须在宏定义**之前**。否则 `int fpclassify(double x);`
 * 会被 `fpclassify(x)` 这个函数式宏展开成 `__builtin_fpclassify(...)`，直接编译错误——
 * 机内 tcc 报的正是 `math.h:63: error: identifier expected`。
 *
 * **宏只在 C 下定义**：C99 要求它们接受任意实参类型（float/double/long double），
 * 故走编译器的内建（clang/GCC 同族，不手拆 IEEE-754 位）；C++ 下若也定义成宏，
 * 会与 libstdc++ 的 `using ::isnan;` 及重载打架——C++ 要的是函数。 */
int fpclassify(double x);
int isnan(double x);
int isinf(double x);
int isfinite(double x);
int isnormal(double x);
int signbit(double x);

/* **不提供宏，只提供函数**——这是实测逼出来的取舍，不是省事：
 * C99 规定这六个是宏，但宏的"泛型"实现只能走编译器内建 `__builtin_*`，
 * 而**机内 tcc 不提供这些内建**：实测 `tcc mathclass_check.c` 报 8 条
 * `unresolved reference to '__builtin_nanf' / '__builtin_isnan' / …`。
 * libc 自己的**函数**在 tcc 里可用，故改成函数：`isnan(f)` 对任意实参类型
 * 经隐式转换到 `double` 即可用，C++ 的 `using ::isnan;` 也拿到了真符号。
 *
 * **诚实边界**：`long double` 的次正规数在转换到 `double` 时可能被归类为 0
 * （80 位扩展精度的最小次正规数小于 `double` 能表示的范围）。分类语义对
 * float/double 完全正确；`long double` 只有这个极端边界不精确。 */

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

#ifdef __cplusplus
}
#endif

#endif /* _MATH_H */
