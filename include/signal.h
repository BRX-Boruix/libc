/* signal.h - BORUIX C 标准库：信号（3P3-2）。
 *
 * ## 符号集 vs 投递能力：两件事，分开说（2026-10 更正）
 *
 * **本头文件定义完整的 POSIX 信号符号集**（SIGHUP..SIGSYS）。POSIX 要求 `<signal.h>` 提供这些
 * **名字**，而且真实程序会用到——**真实触发**：host=boruix 的 cc1 构建里，GCC 自己的
 * `libcody/fatal.cc:53` 直接 `raise (SIGABRT)`，头文件不定义它就直接编不过。
 *
 * **但内核只投递下面这 14 个**（下表带 ✔）：
 *   SIGINT 2 / SIGILL 4 / SIGBUS 7 / SIGFPE 8 / SIGKILL 9 / SIGUSR1 10 / SIGSEGV 11 /
 *   SIGUSR2 12 / SIGPIPE 13 / SIGALRM 14 / SIGTERM 15 / SIGCHLD 17 / SIGCONT 18 / SIGSTOP 19
 * 对**不在**这个集合里的信号，`signal()`/`raise()`/`kill()` 会**如实失败并置 errno**，
 * 绝不假装接受然后什么都不做。**诚实边界**：定义 `SIGABRT` 不等于本系统会投递它——
 * `fatal.cc` 在 `raise` 之后还写了 `exit(2)`，正是"不假定信号一定生效"的写法。
 *
 * （此前本头文件**故意只定义那 14 个**，理由是"声明一个必然失败的信号会诱导程序写出走不通的
 * 分支"。那条理由在**能力声明**上是对的，但用错了地方：POSIX 的**名字空间**是编译期契约，
 * 与运行期能力是两回事——和 `errno.h` 必须给全表是同一条道理。证据驱动更正。）
 *
 * `struct sigaction` 布局必须与 Rust 侧 libc/src/signal.rs 的 #[repr(C)] 逐字段一致
 * （驱动里有 _Static_assert 锚定）。
 */
#ifndef _SIGNAL_H
#define _SIGNAL_H

/* POSIX 要求 <signal.h> 自带 pid_t（kill 的形参就是它）。
 * **来路（第 59 轮，GMP 交叉构建暴露的自身缺陷）**：第 57 轮补 kill 声明时只写了
 * `int kill(pid_t pid, int sig);` 而本头文件当时**不包含任何头**，于是单独 include
 * <signal.h> 的翻译单元会报 `error: unknown type name 'pid_t'`。
 * 教训：**头文件必须自洽**——补声明的同时要补它依赖的类型来源；"名字审计"查不出这类问题。 */
#include "sys/types.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*sighandler_t)(int);

/* C99 要求 <signal.h> 提供 sig_atomic_t（可原子访问的整型）。
 * 来路：host=boruix 的 cc1 构建里 libstdc++ 的 <csignal> 报
 *   `error: no member named 'sig_atomic_t' in the global namespace`。 */
typedef int sig_atomic_t;

#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)

/* ---- 完整 POSIX 信号集（✔ = 内核**真的会投递**；其余只为名字空间完整）---- */
#define SIGHUP     1
#define SIGINT     2   /* ✔ */
#define SIGQUIT    3
#define SIGILL     4   /* ✔ */
#define SIGTRAP    5
#define SIGABRT    6
#define SIGIOT     SIGABRT   /* 别名（Linux） */
#define SIGBUS     7   /* ✔ */
#define SIGFPE     8   /* ✔ */
#define SIGKILL    9   /* ✔ 不可捕获/屏蔽 */
#define SIGUSR1   10   /* ✔ */
#define SIGSEGV   11   /* ✔ */
#define SIGUSR2   12   /* ✔ */
#define SIGPIPE   13   /* ✔ */
#define SIGALRM   14   /* ✔ */
#define SIGTERM   15   /* ✔ */
#define SIGSTKFLT 16
#define SIGCHLD   17   /* ✔ */
#define SIGCONT   18   /* ✔ */
#define SIGSTOP   19   /* ✔ 不可捕获/屏蔽 */
#define SIGTSTP   20
#define SIGTTIN   21
#define SIGTTOU   22
#define SIGURG    23
#define SIGXCPU   24
#define SIGXFSZ   25
#define SIGVTALRM 26
#define SIGPROF   27
#define SIGWINCH  28
#define SIGIO     29
#define SIGPOLL   SIGIO      /* 别名（POSIX） */
#define SIGPWR    30
#define SIGSYS    31

/* 屏蔽集：u64 位图（NSIG=64），与 Rust 侧 libc::signal::sigset_t 同一事实。 */
typedef unsigned long sigset_t;

struct sigaction {
    sighandler_t sa_handler;
    sigset_t     sa_mask;
    int          sa_flags;
    void        *sa_restorer; /* 保留：本系统由内核自动安装 restorer，恒为 0 */
};

/* 信号集操作（POSIX）。**必须用这些**，不要手搓位：
 * 位编码是 `1 << sig`（与内核 SignalSet 同一事实）。 */
int sigemptyset(sigset_t *set);
int sigfillset(sigset_t *set);
int sigaddset(sigset_t *set, int sig);
int sigdelset(sigset_t *set, int sig);
int sigismember(const sigset_t *set, int sig);

/* sigprocmask 的 how 取值（POSIX）。 */
#define SIG_BLOCK   0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

sighandler_t signal(int sig, sighandler_t handler);
int raise(int sig);
/* kill：向进程发信号（POSIX 归属 <signal.h>）。实现在 libc/src/process.rs，此前没有声明
 * （头文件覆盖审计列出）。 */
int kill(pid_t pid, int sig);
int sigaction(int sig, const struct sigaction *act, struct sigaction *oldact);
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);

#ifdef __cplusplus
}
#endif

#endif /* _SIGNAL_H */