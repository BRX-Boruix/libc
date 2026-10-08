/* errno.h —— BORUIX libc 错误码（ADR-010 对齐）。 */
#ifndef _BORUIX_ERRNO_H
#define _BORUIX_ERRNO_H

#ifdef __cplusplus
extern "C" {
#endif

int *__errno_location(void);
#define errno (*__errno_location())

#define EPERM   1
#define ENOENT  2
#define EIO     5
#define ENOMEM  12
#define EACCES  13
#define EEXIST  17
#define EINVAL  22
#define ENOSPC  28
#define ERANGE  34
/* ENOSYS：功能未实现（POSIX）。本系统用它做**如实**拒绝——例如 posix_spawnattr_setflags
 * 的进程组/信号屏蔽/调度优先级旗标（本系统没有那些能力）。 */
#define ENOSYS  38

/* ---- 以下 15 个由**机械门** `libc/tools/audit_errno_sync.py` 一次列出：
 * Rust 侧（libc/src/errno.rs）**一直有**、C 头文件**一直没有** ⇒ 任何 C 程序比较它们都会
 * undeclared。真实触发：GCC 的 `fixincludes/fixlib.c:62` 用 EISDIR 编译时报 undeclared。
 * 数值取自 Rust 侧（那是单点真值；本门会持续校验两侧一致）。 */
#define E2BIG        7
#define ENOEXEC      8
#define EFAULT       14
#define EBUSY        16
#define ENOTDIR      20
#define EISDIR       21
#define ENFILE       23
#define EMFILE       24
#define ESPIPE       29
#define EROFS        30
#define ENAMETOOLONG 36
#define ENOTEMPTY    39
#define ELOOP        40
#define EILSEQ       84
#define EUCLEAN      117
#define EAGAIN  11
#define EBADF   9
#define ENOTSUP 95
/* EINTR（阻塞中的系统调用被信号打断，ADR-051）。取值与 libc/src/errno.rs 的 EINTR 同一事实。
 * 3P6-2 第二波：由 GCC 的真实报错驱动补上——libiberty 的 simple-object.c 用它判断"重试读"。 */
#define EINTR   4

/* ---- 完整 POSIX/Linux errno 符号表（S13：与 libc/src/errno.rs 一一对应） ----
 *
 * **为什么要有这一整张表**：errno.h 按 POSIX 必须定义**完整的符号集**，而不是"本内核会
 * 产生的那几个"。第三方代码会直接比较这些名字——真实触发（3P6-3，libstdc++ 的
 * <system_error> / std::errc）：缺一个 EDEADLK 就报 60 次
 * "'EDEADLK' was not declared in this scope"，整批约 1300 条错误。
 *
 * **诚实边界（S09）**：内核错误码到 errno 的映射（libc/src/errno.rs 的 from_libsys）
 * 只会产生上面那 23 个具名值 + 未知码透传；下面这些宏里**绝大多数本内核永远不会返回**。
 * 定义它们是为了让 C/C++ 程序能编译与比较，**不是**声称本系统会产生这些错误。
 * 数值取 Linux x86_64 的标准编号。机械门 libc/tools/audit_errno_sync.py 校验两侧一致。 */
#define ESRCH             3
#define ENXIO             6
#define ECHILD            10
#define ENOTBLK           15
#define EXDEV             18
#define ENODEV            19
#define ENOTTY            25
#define ETXTBSY           26
#define EFBIG             27
#define EMLINK            31
#define EPIPE             32
#define EDOM              33
#define EDEADLK           35
#define ENOLCK            37
#define ENOMSG            42
#define EIDRM             43
#define ECHRNG            44
#define EL2NSYNC          45
#define EL3HLT            46
#define EL3RST            47
#define ELNRNG            48
#define EUNATCH           49
#define ENOCSI            50
#define EL2HLT            51
#define EBADE             52
#define EBADR             53
#define EXFULL            54
#define ENOANO            55
#define EBADRQC           56
#define EBADSLT           57
#define EBFONT            59
#define ENOSTR            60
#define ENODATA           61
#define ETIME             62
#define ENOSR             63
#define ENONET            64
#define ENOPKG            65
#define EREMOTE           66
#define ENOLINK           67
#define EADV              68
#define ESRMNT            69
#define ECOMM             70
#define EPROTO            71
#define EMULTIHOP         72
#define EDOTDOT           73
#define EBADMSG           74
#define EOVERFLOW         75
#define ENOTUNIQ          76
#define EBADFD            77
#define EREMCHG           78
#define ELIBACC           79
#define ELIBBAD           80
#define ELIBSCN           81
#define ELIBMAX           82
#define ELIBEXEC          83
#define ERESTART          85
#define ESTRPIPE          86
#define EUSERS            87
#define ENOTSOCK          88
#define EDESTADDRREQ      89
#define EMSGSIZE          90
#define EPROTOTYPE        91
#define ENOPROTOOPT       92
#define EPROTONOSUPPORT   93
#define ESOCKTNOSUPPORT   94
#define EPFNOSUPPORT      96
#define EAFNOSUPPORT      97
#define EADDRINUSE        98
#define EADDRNOTAVAIL     99
#define ENETDOWN          100
#define ENETUNREACH       101
#define ENETRESET         102
#define ECONNABORTED      103
#define ECONNRESET        104
#define ENOBUFS           105
#define EISCONN           106
#define ENOTCONN          107
#define ESHUTDOWN         108
#define ETOOMANYREFS      109
#define ETIMEDOUT         110
#define ECONNREFUSED      111
#define EHOSTDOWN         112
#define EHOSTUNREACH      113
#define EALREADY          114
#define EINPROGRESS       115
#define ESTALE            116
#define ENOTNAM           118
#define ENAVAIL           119
#define EISNAM            120
#define EREMOTEIO         121
#define EDQUOT            122
#define ENOMEDIUM         123
#define EMEDIUMTYPE       124
#define ECANCELED         125
#define ENOKEY            126
#define EKEYEXPIRED       127
#define EKEYREVOKED       128
#define EKEYREJECTED      129
#define EOWNERDEAD        130
#define ENOTRECOVERABLE   131
#define ERFKILL           132
#define EHWPOISON         133
#define EWOULDBLOCK       11
#define EDEADLOCK         35
#define EOPNOTSUPP        95

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_ERRNO_H */
