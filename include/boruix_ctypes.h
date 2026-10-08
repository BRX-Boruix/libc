/* BORUIX libc —— 标准头文件。
 *
 * 为未来原生 C 工具链声明 libc 的 C ABI 原型（对应 libc/src 各模块的
 * #[no_mangle] 导出）。类型/宏与 POSIX C 语义对齐。
 * 注：memcpy/memset/memcmp/memmove 由 libsys(builtins) 提供，此处仍声明以
 * 满足标准头文件契约；链接时由 libsys 满足。
 */
#ifndef _BORUIX_CTYPES_H
#define _BORUIX_CTYPES_H

typedef unsigned long size_t;
typedef long ssize_t;
typedef long off_t; /* LP64：与 Rust 侧 libc::ctypes::off_t（c_long）同一事实 */
typedef int c_int;
typedef long c_long;
typedef unsigned long c_ulong;
typedef long long c_longlong;
typedef unsigned long long c_ulonglong;
typedef signed char c_char;
/* wchar_t：**C++ 里是内建类型**（`typedef int wchar_t;` 会报
 * "cannot combine with previous 'int' declaration specifier"）。故只在 C 里 typedef；
 * C++ 用编译器的内建 wchar_t（本目标同为 32 位 int，见 libc/src/wchar.rs 的同一事实）。
 * 来路（2026-10，host=boruix 的 cc1 构建）：libstdc++ 的 <cwchar> 拉进本头文件后立即报错。 */
#ifndef __cplusplus
typedef int wchar_t;
#endif
typedef unsigned c_uint;

/* NULL：C 里用 `((void*)0)` 是惯例，**C++ 里不行**——`(void*)0` 不能隐式转成有类型指针。
 * 真实触发（3P6-3，libstdc++）：`atexit_thread.cc` 的 `single_thread = NULL;` 与
 * `eh_alloc.cc` 的 `first_free_entry->next = NULL;` 都报
 * `invalid conversion from 'void*' to '…*'`——两个"看不懂的 C++ 错"其实都只是这一行宏。
 * C++ 用 `0`（标准做法，可移植）。 */
#ifndef NULL
# ifdef __cplusplus
#  define NULL 0
# else
#  define NULL ((void *)0)
# endif
#endif
#define EOF (-1)
#define RAND_MAX 0x7FFFFFFF

#endif /* _BORUIX_CTYPES_H */
