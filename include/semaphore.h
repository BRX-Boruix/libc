/* semaphore.h —— BORUIX libc 的 POSIX 信号量（sem_*）。
 *
 * 与 csrc/pthread.h 的 bx_sem_t **同一布局**（那边是 freestanding C 侧的实现，
 * 这里是 Rust 版 libc 的实现，两套可互换使用）——故用**共享的**守卫宏，
 * 同时包含两份头文件也不会重复 typedef。
 *
 * 为什么现在才有：csrc 侧一直有实现，但 libc.a（tcc/GCC 链接的那一份）没有导出，
 * 于是 tcc 的 CONFIG_TCC_SEMLOCK（多线程 libtcc 的互斥）无从链接。
 */
#ifndef _BORUIX_SEMAPHORE_H
#define _BORUIX_SEMAPHORE_H

#include "boruix_ctypes.h"

#ifndef BORUIX_BX_SEM_T_DEFINED
#define BORUIX_BX_SEM_T_DEFINED
/* 完整类型（非 opaque），故可作全局/栈对象；每个对象含一个用户态原子计数、一个 epoch
 * 计数器（丢唤醒守卫）与一个内核 SYNC 字 id。 */
typedef struct bx_sem {
    volatile int count;
    volatile unsigned long epoch;
    unsigned long park_id;
    int _init;
} bx_sem_t;
#endif /* BORUIX_BX_SEM_T_DEFINED */

typedef bx_sem_t sem_t;

#ifdef __cplusplus
extern "C" {
#endif

/* 成功返回 0，失败返回 -1 置 errno。pshared != 0（进程间共享）如实返回 ENOTSUP。 */
int sem_init(sem_t* s, int pshared, unsigned int value);
/* 计数 > 0 则递减返回；否则睡到被 post 唤醒（不丢唤醒，见 libc/src/semaphore.rs）。 */
int sem_wait(sem_t* s);
/* 非阻塞；计数为 0 时返回 -1 置 EAGAIN。 */
int sem_trywait(sem_t* s);
int sem_post(sem_t* s);
int sem_destroy(sem_t* s);
int sem_getvalue(sem_t* s, int* val);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_SEMAPHORE_H */
