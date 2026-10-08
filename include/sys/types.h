/* sys/types.h - BORUIX C 标准库：基础类型（3P3-2）。
 *
 * 只补 boruix_ctypes.h 尚未定义的那些；已定义的（size_t/ssize_t/off_t）不重复。
 * 宽度按 x86_64 LP64 取值，与 Rust 侧 libc::ctypes 的对应项同一事实。
 */
#ifndef _SYS_TYPES_H
#define _SYS_TYPES_H

#include "../boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef unsigned int mode_t;   /* u32 */
typedef unsigned long ino_t;   /* u64 */
typedef unsigned long dev_t;   /* u64 */
typedef unsigned long nlink_t; /* u64 */
typedef long blksize_t;        /* i64 */
typedef long blkcnt_t;         /* i64 */
typedef int pid_t;
typedef unsigned int uid_t;
typedef unsigned int gid_t;

#ifdef __cplusplus
}
#endif

#endif /* _SYS_TYPES_H */