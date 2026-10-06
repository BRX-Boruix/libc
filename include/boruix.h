/* boruix.h —— BORUIX libc 聚合头文件。 */
#ifndef _BORUIX_H
#define _BORUIX_H

#include "boruix_ctypes.h"
#include "errno.h"
#include "malloc.h"
#include "stdlib.h"
#include "string.h"
#include "stdio.h"
#include "unistd.h"
#include "time.h"
#include "wchar.h"

/* 由 C 入口桥接（csrc/user_main.c）在调用 main 之前调用：把入口 argc/argv 交给 libc，
 * 使 `environ` / `getenv` 可用。这是**单点**——不要在别处重复注册。 */
void __boruix_init_environ(long argc, const char *const *argv);

/* 开关堆增长诊断：打开后 libc 的 malloc 与 libsys 的 buddy 在每次 brk 扩展时
 * 各打一行 [C|L] <cur_brk> <new_brk>。**默认关闭**。用于定位「两个分配器共用 brk」
 * 一类问题（实测曾用它将「同一区间被重复交给 buddy」抓出来）。 */
void boruix_heap_diag(int on);

#endif /* _BORUIX_H */
