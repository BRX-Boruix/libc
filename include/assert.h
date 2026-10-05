/* assert.h - BORUIX C 标准库：断言（3P3-2）。
 *
 * 定义 NDEBUG 时展开为 ((void)0)（与 C 标准一致）。失败路径调用 __assert_fail()
 * （libc/src/stdlib.rs），它打印 文件:行号: 表达式 后 abort()。
 */
#ifndef _ASSERT_H
#define _ASSERT_H

#ifdef NDEBUG
#define assert(expr) ((void)0)
#else
__attribute__((noreturn)) void __assert_fail(const char *expr, const char *file, unsigned line);
#define assert(expr) \
    ((expr) ? (void)0 : __assert_fail(#expr, __FILE__, __LINE__))
#endif

#endif /* _ASSERT_H */