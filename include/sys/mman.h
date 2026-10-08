/* sys/mman.h - BORUIX C 标准库：内存映射（3P3-2）。
 *
 * 常量编号与内核 mm 层、libsys **同一事实**（单点定义处是内核 mm::user_space，
 * 因为 W^X 策略在那里裁决）。实现见 libc/src/mman.rs。
 */
#ifndef _SYS_MMAN_H
#define _SYS_MMAN_H

#include "../boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

#define PROT_NONE  0
#define PROT_READ  1
#define PROT_WRITE 2
#define PROT_EXEC  4

#define MAP_SHARED    0x01
#define MAP_PRIVATE   0x02
#define MAP_ANONYMOUS 0x20
#define MAP_ANON      MAP_ANONYMOUS
#define MAP_FAILED    ((void *)-1)

void *mmap(void *addr, size_t len, int prot, int flags, int fd, off_t off);
int munmap(void *addr, size_t len);
int mprotect(void *addr, size_t len, int prot);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_MMAN_H */