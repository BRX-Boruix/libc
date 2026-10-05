/* signal.h - BORUIX C 标准库：信号（3P3-2）。
 *
 * 本头文件只声明**基础**接口（signal/raise）；sigaction/sigprocmask 的 Rust 实现已存在，
 * 但它们依赖 sigaction/sigset_t 结构体布局，待与 Rust 侧逐字段核对后再补声明
 * （布局不一致比缺声明更危险，故宁缺勿错）。
 */
#ifndef _SIGNAL_H
#define _SIGNAL_H

typedef void (*sighandler_t)(int);

#define SIG_DFL ((sighandler_t)0)
#define SIG_IGN ((sighandler_t)1)
#define SIG_ERR ((sighandler_t)-1)

#define SIGHUP  1
#define SIGINT  2
#define SIGQUIT 3
#define SIGILL  4
#define SIGABRT 6
#define SIGFPE  8
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGUSR2 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15

sighandler_t signal(int sig, sighandler_t handler);
int raise(int sig);

#endif /* _SIGNAL_H */