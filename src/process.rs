//! 进程函数（C ABI）：exit / getpid / gettid / kill / waitpid / yield。
//!
//! ## getpid/gettid 的实现（threads.md T2-6，S09 如实）
//!
//! 内核提供 `SYS_TASK_GETPID`（0x39，返回所在线程组组长 pid / POSIX 进程 id）与
//! `SYS_TASK_GETTID`（0x38，返回调用线程自身 pid）。本模块直接走内核 syscall，
//! 不再读 ProcFS `/processes/list` 扫 Running（多线程/SMP 下会挑错成员）。
//! 单线程进程 tgid==pid；多线程时组员 getpid==组长 pid、gettid==自身 pid。

use crate::ctypes::c_int;
use crate::errno::{set_errno, from_libsys};

/// `exit(code)`：终止当前进程。永不返回。
///
/// POSIX：`exit` 先按 LIFO 调用 `atexit` 登记的处理函数，再真正退出；`_exit` 不调用。
#[unsafe(no_mangle)]
pub extern "C" fn exit(code: c_int) -> ! {
    crate::stdlib::run_atexit_handlers(code);
    // **必须冲刷 stdio 缓冲**（2026-10 起 stdio 有缓冲层）：不冲刷就是**静默丢数据**——
    // 程序用 printf/fwrite 写的东西会留在用户态缓冲里随进程消失。_exit 按 POSIX
    // 语义**不**冲刷，那是调用方明确要求的"立即终止"。
    let _ = crate::stdio::fflush_all();
    libsys::exit(code)
}

/// `_exit(code)`：立即终止，**不**运行 `atexit` 处理函数、不做任何清理（POSIX 语义）。
#[unsafe(no_mangle)]
pub extern "C" fn _exit(code: c_int) -> ! {
    libsys::exit(code)
}

/// `gettid()`：返回调用线程自身的线程 id（= 内核 pid；组长 pid==tgid、组员 pid==线程 id）。
/// 线程可据此把本线程 id 写进其 `Tcb.tid`。失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn gettid() -> c_int {
    match libsys::gettid() {
        Ok(t) => t as c_int,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// `getpid()`：返回当前进程（线程组组长）PID = 所在组 tgid（POSIX 进程 id）。
/// 走内核 `SYS_TASK_GETPID`（0x39）。失败返回 -1 置 errno。
#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> c_int {
    match libsys::getpid() {
        Ok(t) => t as c_int,
        Err(e) => { set_errno(from_libsys(e)); -1 }
    }
}

/// 查询本进程真实身份（uid/gid/caps）的**单一入口**。
///
/// 四项 get*id 的 POSIX 语义完全一致（查不到才置 errno），故只在这里查一次、只写一份错误路径。
/// 失败时返回 None（**不**返回 0——0 是 root 的合法 uid，兜底就等于伪造身份，S09 不允许）。
fn query_identity() -> Option<libsys::IdentityInfo> {
    match libsys::identity_query() {
        Ok(i) => Some(i),
        Err(e) => {
            set_errno(from_libsys(e));
            None
        }
    }
}

/// getuid()：本进程的**真实** uid。
///
/// **来路（3P6-2 第二波，「整项缺失」类）**：内核早有 identity_query
/// （IdentityInfo 12 字节，uid@0 / gid@4 / caps@8，编译期断言钉死），libsys 也已公开
/// re-export（libsys::identity_query）——缺的只是 libc 这一层。这类「能力已存在、C 面没暴露」
/// 的缺口是**查不出来**的（名字审计只看已导出项），故新增 libc/tools/audit_posix_surface.py
/// 做反向对账（清单 vs 实现）。
///
/// **诚实边界（S09）**：本系统内核只维护**一份** uid/gid，**没有** real/effective 之分，
/// 故 geteuid() 与 getuid() 返回同一值（gid 同理）——这是事实陈述，不是偷懒。
/// POSIX 规定本函数不会失败；内核仅在「无当前进程」的内核上下文返回 PermissionDenied，
/// 用户进程走不到，故此处的 (uid_t)-1 是**防御性**分支而非正常返回。
#[unsafe(no_mangle)]
pub extern "C" fn getuid() -> crate::ctypes::uid_t {
    match query_identity() {
        Some(i) => i.uid,
        None => (-1i32) as crate::ctypes::uid_t,
    }
}

/// geteuid()：有效 uid。本系统无 real/effective 之分，故与 getuid() 同值（见其说明）。
#[unsafe(no_mangle)]
pub extern "C" fn geteuid() -> crate::ctypes::uid_t {
    getuid()
}

/// getgid()：本进程的**真实** gid。
#[unsafe(no_mangle)]
pub extern "C" fn getgid() -> crate::ctypes::gid_t {
    match query_identity() {
        Some(i) => i.gid,
        None => (-1i32) as crate::ctypes::gid_t,
    }
}

/// getegid()：有效 gid。本系统无 real/effective 之分，故与 getgid() 同值。
#[unsafe(no_mangle)]
pub extern "C" fn getegid() -> crate::ctypes::gid_t {
    getgid()
}

/// `nice(inc)`：把调用进程的 nice 值**加上** `inc`，返回**新的** nice（POSIX，声明见 `<unistd.h>`）。
///
/// **返回值可为 -1**（新的 nice 恰好是 -1 时）——POSIX 明确承认这一歧义：调用方须在调用前
/// 把 `errno` 清零再判断。本实现照此，不另造编码。
///
/// 越界由内核**钳制**到 [-20, 19]（与调度器单点一致），不是报错：nice 来自用户态，越界
/// 不应能打挂内核；钳到最近合法档是「最接近用户意图」的确定性行为。
#[unsafe(no_mangle)]
pub extern "C" fn nice(inc: c_int) -> c_int {
    // 先读当前值（pid 0 = 自身）。
    let cur = match libsys::nice_get(0) {
        Ok(n) => n,
        Err(e) => {
            crate::errno::set_errno(crate::errno::from_libsys(e));
            return -1;
        }
    };
    match libsys::nice_set(0, cur + inc) {
        Ok(n) => n,
        Err(e) => {
            crate::errno::set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}

/// `getpriority(which, who)`：取调度优先级（POSIX，声明见 `<sys/resource.h>`）。
///
/// POSIX 关系：`priority = 20 - nice` ⇒ 本函数返回 `20 - nice`（值域 1..=40）。
/// **`which` 只支持 `PRIO_PROCESS`**：本系统无进程组、无多用户（ADR-043），`PRIO_PGRP`/
/// `PRIO_USER` 如实返回 -1 置 `EINVAL`——不假装支持。
#[unsafe(no_mangle)]
pub extern "C" fn getpriority(which: c_int, who: c_int) -> c_int {
    if which != crate::resource::PRIO_PROCESS {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // POSIX：who == 0 表示自身。
    let pid = if who == 0 { 0 } else { who as usize };
    match libsys::nice_get(pid) {
        Ok(n) => crate::resource::PRIORITY_BASE - n,
        Err(e) => {
            crate::errno::set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}

/// `setpriority(which, who, prio)`：设调度优先级（POSIX，声明见 `<sys/resource.h>`）。
///
/// `nice = 20 - prio`（与 `getpriority` 同一关系）。`which` 只支持 `PRIO_PROCESS`（同上）。
/// **改他人需 `CAP_SYSTEM`**（内核单点判定），越权如实 `EPERM`。
#[unsafe(no_mangle)]
pub extern "C" fn setpriority(which: c_int, who: c_int, prio: c_int) -> c_int {
    if which != crate::resource::PRIO_PROCESS {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let pid = if who == 0 { 0 } else { who as usize };
    match libsys::nice_set(pid, crate::resource::PRIORITY_BASE - prio) {
        Ok(_) => 0,
        Err(e) => {
            crate::errno::set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}

/// `getgroups(size, list)`：查询调用进程的**补充组**集合（POSIX，声明见 `<unistd.h>`）。
///
/// **契约（POSIX）**：
/// - `size == 0`（`list` 可为 NULL）⇒ 只返回**所需条数**（探测形态）；
/// - 否则写入至多 `size` 个并返回**实际条数**；
/// - `size` 不足 ⇒ 返回 **-1** 置 `EINVAL`（POSIX 规定；**不**截断、不写半个列表）。
///
/// **为什么现在才有**：内核此前只有 `SYS_TASK_GROUPS_SET`（只回传设置后的结果），
/// **没有独立查询入口** ⇒ 用户态无法知道自己当前的补充组，libc 侧因此只能把 `getgroups`
/// 登记为整项缺失。本轮补了内核动词 `SYS_TASK_GROUPS_GET`（0x3E）才有这条出口。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgroups(size: c_int, list: *mut crate::ctypes::gid_t) -> c_int {
    if size < 0 {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    if size == 0 {
        // 探测：只问条数（内核允许 a1 == 0 当且仅当容量为 0）。
        return match libsys::groups_get(&mut []) {
            Ok(n) => n as c_int,
            Err(e) => {
                crate::errno::set_errno(crate::errno::from_libsys(e));
                -1
            }
        };
    }
    if list.is_null() {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let out = unsafe { core::slice::from_raw_parts_mut(list, size as usize) };
    match libsys::groups_get(out) {
        Ok(n) if n <= size as usize => n as c_int,
        // 缓冲不足：内核只写了能容纳的部分，但 POSIX 要求此时返回 -1/EINVAL。
        Ok(_) => {
            crate::errno::set_errno(crate::errno::EINVAL);
            -1
        }
        Err(e) => {
            crate::errno::set_errno(crate::errno::from_libsys(e));
            -1
        }
    }
}

/// getppid()：父进程 pid。
///
/// **来路（3P6-2 第二波「整项缺失」类）**：父 pid 由内核经 procfs 暴露
/// （`vfs/src/procfs.rs` 的 ppid 字段），libsys 已有 `ps_list()` 把它解析成 `PsEntry`——
/// 缺的只是 libc 这一层。
///
/// **诚实边界（S09）**：数据来自 procfs 的**一次快照读**；返回的是快照时刻的真实父子关系，
/// 不是缓存、不是猜测。POSIX 规定本函数不会失败，故查不到时返回 0（防御性分支；正常进程
/// 一定在进程表里）。
#[unsafe(no_mangle)]
pub extern "C" fn getppid() -> c_int {
    let me = match libsys::getpid() {
        Ok(p) => p,
        Err(e) => {
            set_errno(from_libsys(e));
            return 0;
        }
    };
    match libsys::ps_list() {
        Ok(list) => {
            for e in list.iter() {
                if e.pid as u64 == me {
                    return e.ppid as c_int;
                }
            }
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            0
        }
    }
}

/// `kill(pid, sig)`：向进程发信号。返回 0 或 -1（置 errno）。
///
/// **POSIX 的 `pid` 三档语义在本系统没有完全对应物**（`getpgrp`/`setpgid`/`setsid` 属
/// 登记缺失项，内核**没有进程组概念**，见 `waitpid` 的说明）。故逐档**如实**处理，
/// 不把"做不到"混成"没这个进程"：
///
/// - `pid > 0`：直接转发（目标不存在时内核给 `InvalidParam` ⇒ EINVAL）；
/// - `pid == 0`：POSIX 是「发给**调用者所在进程组**」。本系统每个进程自成一个组
///   （无组可加入），组内唯一成员就是自己 ⇒ **等价于 `kill(getpid(), sig)`**。
///   这不是近似：在"无组"模型下这就是真值。此前把它当 pid=0 直接下发，内核查不到
///   该 pid ⇒ 返回 EINVAL——连「`kill(0, 0)` 探活自己」这个最常见用法都失败；
/// - `pid < 0`：POSIX 是「发给进程组 `|pid|`」（`-1` = 所有有权限的进程）。本系统做不到
///   ⇒ 如实 **ENOTSUP**，**不**静默降级成"只发给自己"（那会让调用方以为整组都收到了）。
///
/// （语义忠实性审查第七轮。）
#[unsafe(no_mangle)]
pub extern "C" fn kill(pid: c_int, sig: c_int) -> c_int {
    if pid < 0 {
        set_errno(crate::errno::ENOTSUP);
        return -1;
    }
    let target = if pid == 0 {
        // getpid 失败时已置 errno，直接上抛。
        let me = getpid();
        if me < 0 {
            return -1;
        }
        me
    } else {
        pid
    };
    match libsys::kill(target as u64, sig as u64) {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `waitpid(pid, status, options)`：等待子进程退出。
///
/// 返回被收尸子进程的 **pid**（POSIX 语义；失败返回 -1 并置 errno）。退出码
/// 写入 `*status` 高 8 位（WEXITSTATUS 语义）。
///
/// - `pid`（语义忠实性审查登记项⑧）：内核**本来就**支持按 pid 等
///   （`task::scheduler::waitpid_inner` 的单目标分支），此前本层把 pid 丢掉、一律退化成
///   「等任意子」——那会让 `waitpid(specific_pid)` 收走**别的**子进程。现在：
///   `pid > 0` 等该子进程；`pid == -1` 等任意子进程；
///   `pid == 0`（同进程组）与 `pid < -1`（进程组 |pid|）本系统**无进程组** ⇒ 如实 ENOTSUP。
/// - `options`：`WNOHANG` 支持；其余非 0 如实 -1/ENOTSUP。
#[unsafe(no_mangle)]
pub extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    // POSIX 的 pid 语义（语义忠实性审查登记项⑧）：内核**本来就**支持按 pid 等
    // （`task::scheduler::waitpid_inner` 的单目标分支），此前本层把 pid 丢掉、
    // 一律退化成「等任意子」——那会让 `waitpid(specific_pid)` 收走**别的**子进程。
    let target: Option<u64> = if pid == -1 {
        None // 等任意子进程（内核的 WAIT_ANY 哨兵）
    } else if pid > 0 {
        Some(pid as u64)
    } else {
        // pid == 0（同进程组）与 pid < -1（进程组 |pid|）：本系统**无进程组** ⇒ 如实 ENOTSUP。
        set_errno(crate::errno::ENOTSUP);
        return -1;
    };
    let nonblock = options & crate::unistd::WNOHANG != 0;
    if options != 0 && !nonblock {
        set_errno(crate::errno::ENOTSUP);
        return -1;
    }
    // WNOHANG：非阻塞轮询。本系统内核支持「有界等待」——到期子进程仍在运行则如实返回
    // `WouldBlock`（见 kernel syscall.rs 的 sys_task_wait 文档），故这里用 1ns 超时实现
    // 「立即返回」。**诚实边界**：受内核定时器粒度限制，1ns 可能短暂等待（远小于一次调度片），
    // 但语义正确（没有子进程已退出就返回 0）。
    let res = if nonblock {
        match target {
            Some(t) => libsys::waitpid(t, 1),
            None => libsys::waitpid_any_timeout(1),
        }
    } else {
        match target {
            Some(t) => libsys::waitpid(t, 0),
            None => libsys::waitpid_any(),
        }
    };
    match res {
        Ok(wr) => {
            if !status.is_null() {
                unsafe {
                    *status = ((wr.code & 0xFF) as c_int) << 8;
                }
            }
            wr.pid as c_int
        }
        // WNOHANG 没等到 ⇒ 0（POSIX）；阻塞路径拿到 WouldBlock 是内核「无可切对象」的
        // 如实回报，按错误上抛（EAGAIN）。
        Err(libsys::Error::WouldBlock) if nonblock => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `wait(status)`：等待**任意**子进程退出（POSIX）。等价于 `waitpid(-1, status, 0)`。
///
/// **为什么它是 3P6-2 的必补项（真实报错驱动，不是预猜）**：GCC 的 configure 用
/// `AC_HEADER_SYS_WAIT` 探测本头文件（日志短语 `sys/wait.h that is POSIX.1 compatible`），
/// 该探测程序体里调用的正是 `wait(&s)`。缺 `wait` 使探测失败——实测日志：
///   `checking for sys/wait.h that is POSIX.1 compatible... no`
/// 于是 `HAVE_SYS_WAIT_H` 未定义，libiberty/pex-unix.c **不包含** <sys/wait.h>，
/// 紧接着 `pex-unix.c:164` 报 `call to undeclared function 'waitpid'`。
///
/// 也就是说：这**一条**缺失同时制造了「头文件不兼容」与「waitpid 未声明」两个报错，
/// 而 waitpid 的实现一直都在（本文件上面就是）——真正缺的只有 `wait` 本身。
#[unsafe(no_mangle)]
pub extern "C" fn wait(status: *mut c_int) -> c_int {
    // pid = -1（等任意子）与内核能力一致；options = 0（阻塞等待）。
    // 直接复用 waitpid：不另写一份收尸逻辑（S15 单点），也不伪造 pid 精确匹配。
    waitpid(-1, status, 0)
}

/// `fork()`：创建一个子进程（COW 语义）。
///
/// # 为什么这不是 POSIX `fork()` 的逐字复刻（诚实声明，S39）
///
/// 本实现的**语义**是 POSIX fork：调用一次、返回两次；父收子 pid（> 0）、子收 0；
/// 失败父收 -1 置 errno。子进程从**本函数内部**的返回点继续执行，拥有父在调用
/// 时刻的全部内存映像（写时复制）与 fd/cwd/identity 拷贝。
///
/// 但有三条**明确的差异**，调用方必须知晓：
///
/// 1. **多线程父进程被内核拒绝**（`ENOTSUP`）。POSIX fork 在子进程内只保留调用
///    线程，其余线程消失；那些线程可能正持有互斥锁/维护着全局不变式，子进程里
///    这些锁将永久无人释放。ADR-038 决策 4 选择**如实拒绝**而非静默产出语义残缺
///    的子进程——宁可让调用方收到错误，也不给它一个会随机死锁的进程。
/// 2. **无 atexit / 无 stdio 缓冲复制**：与 `exit()` 一致，本实现无 atexit 处理器；
///    libc 的 stdio 缓冲若需在 fork 后清空，由调用方自行 `fflush` 处理。
/// 3. **信号处置**按 ADR-034/ADR-038 决策 5：子进程继承父的 restorer 地址与信号
///    状态快照，但**不**继承父的未决信号掩码中的"正在处理"重入守卫。
///
/// # 返回
///
/// - 父进程：新子进程的 pid（`> 0`）；
/// - 子进程：`0`；
/// - 失败：`-1` 并置 `errno`（父进程内，子进程不存在）。
///
/// # 实现注意（为何函数体里没有锁、没有 `static mut`）
///
/// 本函数**会返回两次**。任何在返回点之前持有、且预期在返回点之后仍一致的资源
/// （全局锁、`static mut` 缓存、可变借用）都会在子进程里处于"被父进程持有"的
/// 假象状态。故本实现刻意保持**无状态**：只调一个 syscall 并立即返回，不触碰任何
/// 全局可变状态。`errno` 的设置也只在失败路径（那时代码只在父进程内执行，子进程
/// 从未被创建），故不存在"子进程误继承父的 errno"的问题。
#[unsafe(no_mangle)]
pub extern "C" fn fork() -> c_int {
    // 三个保留参数显式钉为 0（继承父当前 RIP/RSP）——语义写在调用点，
    // 不依赖调用方记得传 0（S13：不留魔法值）。
    match libsys::derive_inherit() {
        Ok(pid) => pid as c_int,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `yield()`：主动让出 CPU。
#[unsafe(no_mangle)]
pub extern "C" fn yield_sys() -> c_int {
    match libsys::yield_now() {
        Ok(_) => 0,
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}
