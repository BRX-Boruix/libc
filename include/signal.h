/* signal.h - BORUIX C 标准库：信号（3P3-2）。
 *
 * **只声明本系统真正支持的信号**。内核 `task::signals` 定义的集合是：
 *   SIGINT 2 / SIGILL 4 / SIGBUS 7 / SIGFPE 8 / SIGKILL 9 / SIGUSR1 10 / SIGSEGV 11 /
 *   SIGUSR2 12 / SIGPIPE 13 / SIGALRM 14 / SIGTERM 15 / SIGCHLD 17 / SIGCONT 18 / SIGSTOP 19
 * 实测内核**没有** SIGHUP/SIGQUIT/SIGABRT——故本头文件**不声明**它们：声明一个必然失败的
 * 信号只会诱导程序写出走不通的分支（S09 不伪造）。
 *
 * `struct sigaction` 布局必须与 Rust 侧 libc/src/signal.rs 的 #[repr(C)] 逐字段一致
 * （驱动里有 _Static_assert 锚定）。
 */
#ifndef _SIGNAL_H
#define _SIGNAL_H

typedef void (*sighandler_t)(int);

#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)

#define SIGINT  2
#define SIGILL  4
#define SIGBUS  7
#define SIGFPE  8
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGUSR2 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19

/* 屏蔽集：u64 位图（NSIG=64），与 Rust 侧 libc::signal::sigset_t 同一事实。 */
typedef unsigned long sigset_t;

struct sigaction {
    sighandler_t sa_handler;
    sigset_t     sa_mask;
    int          sa_flags;
    void        *sa_restorer; /* 保留：本系统由内核自动安装 restorer，恒为 0 */
};

/* sigprocmask 的 how 取值（POSIX）。 */
#define SIG_BLOCK   0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

sighandler_t signal(int sig, sighandler_t handler);
int raise(int sig);
int sigaction(int sig, const struct sigaction *act, struct sigaction *oldact);
int sigprocmask(int how, const sigset_t *set, sigset_t *oldset);

#endif /* _SIGNAL_H */