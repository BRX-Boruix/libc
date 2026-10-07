/* sys/utsname.h - BORUIX C 标准库：系统标识。实现见 libc/src/posix_batch5.rs。
 *
 * **逐字段真值来源**（S09）：sysname="Boruix"；**nodename 为空串**（本系统没有主机名概念，
 * 已核实无数据源，不编 localhost）；release 来自内核 INFO_VERSION；version 复用同一版本号
 * （本系统没有独立构建标识串）；machine="x86_64"（编译期事实）。
 */
#ifndef _SYS_UTSNAME_H
#define _SYS_UTSNAME_H

#ifdef __cplusplus
extern "C" {
#endif

struct utsname {
    char sysname[65];
    char nodename[65];
    char release[65];
    char version[65];
    char machine[65];
};

int uname(struct utsname *buf);

#ifdef __cplusplus
}
#endif

#endif /* _SYS_UTSNAME_H */