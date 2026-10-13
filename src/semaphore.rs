//! POSIX 信号量（`sem_*`）—— 用户态原子计数 + 内核 SYNC 字作 park/unpark。
//!
//! **为什么现在才有**：`csrc/pthread_sync.c` 侧一直有实现（供 csrc 的 freestanding C 程序用），
//! 但 `libc.a`（Rust 侧——tcc/GCC 链接的那一份）**没有导出** `sem_*`。tcc 的
//! `CONFIG_TCC_SEMLOCK`（多线程 libtcc 的互斥）需要它，故此前只能**如实拒绝**
//! 「多线程共用 libtcc」。本模块把同一套语义落到 `libc.a`。
//!
//! **语义与 csrc 严格一致**（那边是已实测工作的参照，T2-5 的教训也照搬）：
//! `count` + `epoch` 两个用户态原子 + 一个内核 SYNC 字。**`epoch` 是丢唤醒守卫**：
//! `sem_wait` 在决定入睡**之前**快照 epoch，`sem_post` 先把新 epoch 写进 SYNC 字**再**唤醒
//! ⇒ 「决定入睡」与「已经 post」之间到达的 post 会让 `sync_wait` 立即返回（字值已不等于
//! 快照的 expected）而不是睡死。若拿 0 当 park 值，那个窗口里的唤醒会被静默丢掉。

use core::ffi::{c_int, c_uint};
use core::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use crate::errno::{from_libsys, set_errno, EINVAL};

/// 与 `csrc/pthread.h` 的 `bx_sem_t` **逐字段同布局**（`repr(C)`，size = 32，两套实现可互换）。
#[repr(C)]
pub struct bx_sem_t {
    /// 当前计数；`> 0` 时 `sem_wait` 直接递减返回。
    pub count: AtomicI32,
    /// 每次 post 递增；镜像到内核 SYNC 字，作丢唤醒守卫。
    pub epoch: AtomicU64,
    /// 内核 SYNC 字 id（init 创建、destroy 销毁）。init 之后只读。
    pub park_id: u64,
    /// 是否已 init；未 init 的 wait/post 如实返回 -1，不假装成功。
    pub _init: i32,
}

/// `sem_init(sem, pshared, value)`：创建计数为 `value` 的信号量。
///
/// **诚实边界**：`pshared != 0`（进程间共享）**不支持**——本实现把 park_id 放在调用方的
/// 结构里，跨进程需要把该结构本身放进共享内存并保证创建者存活；内核 SYNC 字可跨进程传递，
/// 但本函数不做那层安排。非 0 时如实返回 -1 置 `ENOTSUP`（与 `csrc` 侧同样忽略 `pshared`
/// 的行为相比，这里选择**如实拒绝**而不是假装支持）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_init(s: *mut bx_sem_t, pshared: c_int, value: c_uint) -> c_int {
    if s.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if pshared != 0 {
        set_errno(crate::errno::ENOTSUP);
        return -1;
    }
    match libsys::sync_create(0) {
        Ok(id) => {
            let p = unsafe { &mut *s };
            p.count.store(value as i32, Ordering::Relaxed);
            p.epoch.store(0, Ordering::Relaxed);
            p.park_id = id;
            p._init = 1;
            0
        }
        Err(e) => {
            set_errno(from_libsys(e));
            -1
        }
    }
}

/// `sem_wait(sem)`：计数 `> 0` 则递减返回；否则睡到被 post 唤醒（**绝不丢唤醒**，见模块头）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_wait(s: *mut bx_sem_t) -> c_int {
    if s.is_null() || unsafe { (*s)._init } == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let p = unsafe { &*s };
    loop {
        let c = p.count.load(Ordering::Relaxed);
        if c > 0
            && p.count
                .compare_exchange_weak(c, c - 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return 0;
        }
        // 先快照 epoch，再确认计数仍为 0——顺序与 csrc 一致。
        let e = p.epoch.load(Ordering::Relaxed);
        if p.count.load(Ordering::Relaxed) > 0 {
            continue; // 快照与决定之间来了 post：重试而不是睡
        }
        // 值仍等于快照则睡；post 已改字值则立即返回（这正是丢唤醒守卫）。
        let _ = libsys::sync_wait(p.park_id, e, 0);
    }
}

/// `sem_trywait(sem)`：非阻塞；计数为 0 时返回 -1 置 `EAGAIN`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_trywait(s: *mut bx_sem_t) -> c_int {
    if s.is_null() || unsafe { (*s)._init } == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let p = unsafe { &*s };
    let c = p.count.load(Ordering::Relaxed);
    if c > 0
        && p.count
            .compare_exchange_weak(c, c - 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        return 0;
    }
    set_errno(crate::errno::EAGAIN);
    -1
}

/// `sem_post(sem)`：计数 +1；**先**把新 epoch 写进 SYNC 字**再**唤醒一个等待者。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_post(s: *mut bx_sem_t) -> c_int {
    if s.is_null() || unsafe { (*s)._init } == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let p = unsafe { &*s };
    p.count.fetch_add(1, Ordering::AcqRel);
    let e = p.epoch.fetch_add(1, Ordering::AcqRel) + 1;
    // 先改字值后唤醒：决定入睡窗口里到达的 post 会让等待者的 sync_wait 立即返回。
    let _ = libsys::sync_wake(p.park_id, e, 1);
    0
}

/// `sem_destroy(sem)`：销毁内核 SYNC 字并清 init 标志。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_destroy(s: *mut bx_sem_t) -> c_int {
    if s.is_null() || unsafe { (*s)._init } == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let p = unsafe { &*s };
    let _ = libsys::sync_delete(p.park_id);
    unsafe { (*s)._init = 0 };
    0
}

/// `sem_getvalue(sem, val)`：把当前计数写入 `val`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_getvalue(s: *mut bx_sem_t, val: *mut c_int) -> c_int {
    if s.is_null() || val.is_null() || unsafe { (*s)._init } == 0 {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { *val = (*s).count.load(Ordering::Relaxed) };
    0
}