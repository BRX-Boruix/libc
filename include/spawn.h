/* spawn.h - BORUIX C 标准库：posix_spawn 家族（POSIX.1-2008）。
 * 实现见 libc/src/spawn.rs。
 *
 * **为什么它对 BORUIX 特别重要**：GCC 的 driver 用 exec 替换去跑 cc1/as/ld，而本系统没有
 * 「替换当前进程映像」的 syscall；POSIX 的 posix_spawn **本来就不要求替换**，故它是宿主端口的
 * 正确基座（底层是 SYS_TASK_SPAWN，即 libsys::exec_path）。
 *
 * **诚实边界（S09）**：
 *  - 参数含空格/制表符 ⇒ 如实返回 EINVAL（本 ABI 是「argv[0] = 整条命令行」且只按空白切词，
 *    不支持引号，故无法无损表达）；
 *  - `envp` **被忽略**：本系统的环境每次派生由内核从权威状态重建（3P4-2b 未实现）；
 *  - `attrp` 只支持 `POSIX_SPAWN_RESETIDS`（且在本系统里是无操作），其余旗标 ⇒ ENOSYS；
 *  - `file_actions` 在**父进程里**施加后还原（没有 fork+exec 两步）⇒ 多线程父进程有可见窗口。
 *
 * 下面两个结构是 **BORUIX 自有布局**（POSIX 只要求它们是不透明类型）。
 */
#ifndef _SPAWN_H
#define _SPAWN_H

#include "sys/types.h"
#include "signal.h"

#ifdef __cplusplus
extern "C" {
#endif

#define POSIX_SPAWN_RESETIDS        0x01
#define POSIX_SPAWN_SETPGROUP       0x02
#define POSIX_SPAWN_SETSIGDEF       0x04
#define POSIX_SPAWN_SETSIGMASK      0x08
#define POSIX_SPAWN_SETSCHEDPARAM   0x10
#define POSIX_SPAWN_SETSCHEDULER    0x20

struct __boruix_spawn_action {
    int   tag;      /* 0=open 1=close 2=dup2 */
    int   fd;
    char *path;     /* open 用 */
    int   oflag;    /* open 用 */
    unsigned mode;  /* open 用 */
    int   newfd;    /* dup2 用 */
    int   _pad;
};

typedef struct {
    int count;
    int _pad;
    struct __boruix_spawn_action actions[16];
} posix_spawn_file_actions_t;

typedef struct {
    short flags;
    short _pad;
    int   pgroup;
    sigset_t sigmask;
    sigset_t sigdefault;
    int   sched_policy;
    int   sched_priority;
    int   _reserved[8];
} posix_spawnattr_t;

int posix_spawn(pid_t *pid, const char *path,
                const posix_spawn_file_actions_t *file_actions,
                const posix_spawnattr_t *attrp,
                char *const argv[], char *const envp[]);
int posix_spawnp(pid_t *pid, const char *file,
                 const posix_spawn_file_actions_t *file_actions,
                 const posix_spawnattr_t *attrp,
                 char *const argv[], char *const envp[]);

int posix_spawn_file_actions_init(posix_spawn_file_actions_t *file_actions);
int posix_spawn_file_actions_destroy(posix_spawn_file_actions_t *file_actions);
int posix_spawn_file_actions_addopen(posix_spawn_file_actions_t *file_actions,
                                     int fildes, const char *path, int oflag, mode_t mode);
int posix_spawn_file_actions_addclose(posix_spawn_file_actions_t *file_actions, int fildes);
int posix_spawn_file_actions_adddup2(posix_spawn_file_actions_t *file_actions,
                                     int fildes, int newfildes);

int posix_spawnattr_init(posix_spawnattr_t *attr);
int posix_spawnattr_destroy(posix_spawnattr_t *attr);
int posix_spawnattr_getflags(const posix_spawnattr_t *attr, short *flags);
int posix_spawnattr_setflags(posix_spawnattr_t *attr, short flags);

#ifdef __cplusplus
}
#endif

#endif /* _SPAWN_H */