//! printf 家族与缓冲流（FILE*）I/O（C ABI）。
//!
//! ## 真实数据链路（S06）
//!
//! - \`printf\`/\`puts\` 最终写到 fd 1（STDOUT）；\`fprintf\`/\`fwrite\` 写到 FILE
//!   （经内核 fd 读写 syscall）。
//! - 可变参数经 \`c_variadic\`（Rust nightly 特性）读取，与 C 调用约定一致。
//!
//! ## FILE* 流
//!
//! 提供最小 \`FILE\` 结构。默认**无缓冲直写**（本内核页缓存写即落盘，无缓冲
//! 期收益；S32 无优化无数据），结构保留缓冲字段以备将来引入。
//!
//! ## 错误处理（S09/S18）
//!
//! 系统调用失败如实设置 errno 并返回错误（-1/NULL）；FILE 打开失败返回 NULL，
//! fd 在 fclose 时关闭，错误路径同样释放。

use core::cell::UnsafeCell;
use core::ffi::c_char;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// 使 `UnsafeCell<FILE>` 可作为 Sync static（单进程模型 + STREAM_LOCK 保护）。
struct SyncStream(UnsafeCell<FILE>);
unsafe impl Sync for SyncStream {}
impl SyncStream {
    const fn new(f: FILE) -> Self { SyncStream(UnsafeCell::new(f)) }
    fn get(&self) -> *mut FILE { self.0.get() }
}

use crate::ctypes::{size_t, ssize_t, c_int, c_void, c_long, EOF};
use crate::errno::{set_errno, from_libsys, EINVAL, EBADF};
use crate::stdio_format::{FmtSink, Spec, Length, Conv, emit_int, emit_str, emit_char, parse_and_format};

// ---------- FILE 结构 ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FmMode { Read, Write, Append, ReadWrite, AppendRead }

impl FmMode {
    /// 本流是否**可读**。**单点定义（S15）**：此前各处直接写 `f.mode != FmMode::Read`，
    /// 加入 `ReadWrite`/`AppendRead` 后必须走这里——否则新增变体会静默落在错误一侧
    /// （例如 `tmpfile` 的读写流被当成只写流，`fread` 全部失败）。
    #[inline]
    pub fn can_read(self) -> bool {
        matches!(self, FmMode::Read | FmMode::ReadWrite | FmMode::AppendRead)
    }
    /// 本流是否**可写**（只有纯读流不可写）。
    #[inline]
    pub fn can_write(self) -> bool {
        !matches!(self, FmMode::Read)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FmBufMode { None, Line, Full }

#[repr(C)]
pub struct FILE {
    pub fd: u64,
    pub mode: FmMode,
    pub buf_mode: FmBufMode,
    pub eof: bool,
    pub error: bool,
    pub buf_ptr: *mut u8,
    pub buf_len: usize,
    pub buf_pos: usize,
    pub closed: bool,
    /// 1 字节 pushback 槽（ungetc / fscanf 回退）。-1 表示空。
    pub pushback: i32,
    /// **内存源**（`sscanf` 用）：非 null 时扫描从这个缓冲区读，而不是 fd。
    ///
    /// 为什么给 FILE 加源而不是另写一套扫描器：解析逻辑必须**单点定义**（S15）——
    /// 复制第二套 sscanf 解析迟早与 fscanf 分叉。C 侧 `FILE` 是不透明类型
    /// （`typedef struct FILE FILE;`），故加字段不破坏 ABI。
    pub str_src: *const u8,
    pub str_len: usize,
    pub str_pos: usize,
}

/// 全局锁：保护标准流初始化与输出（单进程模型下防重入）。
static STREAM_LOCK: AtomicBool = AtomicBool::new(false);
fn lock() {
    while STREAM_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}
fn unlock() { STREAM_LOCK.store(false, Ordering::Release); }

/// 标准流（惰性初始化）。
#[unsafe(no_mangle)]
pub static mut stdin: *mut FILE = core::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut stdout: *mut FILE = core::ptr::null_mut();
#[unsafe(no_mangle)]
pub static mut stderr: *mut FILE = core::ptr::null_mut();


// ---------- tmpfile 的**延迟删除**登记 ----------
//
// **为什么不立即 unlink**（POSIX 允许立即删，glibc 就是那么做的）：那要求「打开后删除」的
// 句柄仍能正常读写。**实测本内核的 VFS 不支持「unlink 之后再写入」**——同一程序里
// `mkstemp` → `unlink` → `fwrite`（返回 13，内核接受）→ `fseek(fd,0,SEEK_SET)` → `fread`
// 得到 **0 字节 + EOF**：数据在 unlink 之后写不进去（诊断输出见 tools/3psrc/libcc1）。
// 故改为**延迟删除**：`fclose` 时删除；程序未关流就退出时，由一次性登记的 `atexit` 处理器兜底。
// 两条路合起来与 POSIX 的「关闭**或**进程终止时自动消失」一致。
const TMP_MAX: usize = 16;
/// `(占用, fd, 路径指针 as usize)`。指针存成 usize 是为了满足 `Mutex<T>: Send`（同 stdlib 的
/// `ExitEntry::WithArg`）。用显式 `占用` 位而不是拿 fd==0 当空标记——fd 0 是合法的标准输入。
static TMP_FILES: spin::Mutex<[(bool, i32, usize); TMP_MAX]> = spin::Mutex::new([(false, 0, 0); TMP_MAX]);
static TMP_CLEANUP_REGISTERED: AtomicBool = AtomicBool::new(false);

/// 登记一个待删除的临时文件路径（所有权转移给登记表）。表满返回 false。
fn tmp_register(fd: i32, path: *mut c_char) -> bool {
    let mut t = TMP_FILES.lock();
    for slot in t.iter_mut() {
        if !slot.0 {
            *slot = (true, fd, path as usize);
            return true;
        }
    }
    false
}

/// `fclose` 钩子：若该 fd 是 `tmpfile` 建的，删除其文件并释放路径。非 tmpfile 的 fd 无副作用。
unsafe fn tmp_release(fd: i32) {
    let mut ptr: usize = 0;
    {
        let mut t = TMP_FILES.lock();
        for slot in t.iter_mut() {
            if slot.0 && slot.1 == fd {
                ptr = slot.2;
                *slot = (false, 0, 0);
                break;
            }
        }
    }
    if ptr != 0 {
        let p = ptr as *mut c_char;
        if let Ok(s) = core::str::from_utf8(unsafe { cstr_bytes(p) }) {
            let _ = libsys::unlink(s);
        }
        crate::malloc::free(p as *mut u8);
    }
}

/// 进程退出兜底（`atexit` 登记一次）：删除所有仍未随 `fclose` 释放的临时文件。
extern "C" fn tmp_cleanup_at_exit() {
    for i in 0..TMP_MAX {
        let (used, ptr) = {
            let mut t = TMP_FILES.lock();
            let s = (t[i].0, t[i].2);
            t[i] = (false, 0, 0);
            s
        };
        if used && ptr != 0 {
            let p = ptr as *mut c_char;
            if let Ok(s) = core::str::from_utf8(unsafe { cstr_bytes(p) }) {
                let _ = libsys::unlink(s);
            }
            crate::malloc::free(p as *mut u8);
        }
    }
}
/// 初始化标准流（幂等，可多次调用）。
pub fn stdio_init() {
    lock();
    unsafe {
        static STREAM_STDIN: SyncStream = SyncStream::new(FILE {
            fd: 0, mode: FmMode::Read, buf_mode: FmBufMode::None, eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false, pushback: -1,
            str_src: core::ptr::null(), str_len: 0, str_pos: 0,
        });
        static STREAM_STDOUT: SyncStream = SyncStream::new(FILE {
            fd: 1, mode: FmMode::Write, buf_mode: FmBufMode::Line, eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false, pushback: -1,
            str_src: core::ptr::null(), str_len: 0, str_pos: 0,
        });
        static STREAM_STDERR: SyncStream = SyncStream::new(FILE {
            fd: 2, mode: FmMode::Write, buf_mode: FmBufMode::None, eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false, pushback: -1,
            str_src: core::ptr::null(), str_len: 0, str_pos: 0,
        });
        if stdin.is_null() {
            stdin = STREAM_STDIN.get();
            stdout = STREAM_STDOUT.get();
            stderr = STREAM_STDERR.get();
        }
    }
    unlock();
}


/// 分配并初始化一个 FILE（fopen/fdopen/freopen 共用，S15 单点）。
unsafe fn alloc_file(fd: u64, fm: FmMode) -> *mut FILE {
    let fp = crate::malloc::malloc(core::mem::size_of::<FILE>()) as *mut FILE;
    if fp.is_null() {
        set_errno(crate::errno::ENOMEM);
        return core::ptr::null_mut();
    }
    unsafe {
        core::ptr::write(fp, FILE {
            fd, mode: fm,
            // 写流默认**全缓冲**（2026-10 实测驱动：内核写直通 8.9ms/次）。
            // 读流本轮不缓冲（读侧要处理与 lseek/fseek 的位置语义，另案）。
            buf_mode: if fm.can_write() { FmBufMode::Full } else { FmBufMode::None },
            eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false,
            pushback: -1,
            str_src: core::ptr::null(), str_len: 0, str_pos: 0,
        });
        // 登记以便 fflush(NULL) 与 exit 冲刷全部流。表满则如实放弃（该流仍可用，
        // 只是不会被 exit 自动冲刷——返回 false 不假装登记成功）。
        let _ = stream_register(fp);
    }
    fp
}

/// `fdopen(fd, mode)`：把**已打开**的 fd 包成 FILE*（不重新打开、不动文件偏移）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fdopen(fd: crate::ctypes::c_int, mode: *const c_char) -> *mut FILE {
    if mode.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    // fdopen **不重新打开**文件，故模式串只决定 FILE 的读写能力，旗标丢弃（S09 明确）。
    let (fm, _flags) = match parse_mode(mode) {
        Some(m) => m,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    unsafe { alloc_file(fd as u64, fm) }
}

/// `freopen(path, mode, fp)`：把 fp 重新绑到 path，返回**同一个** FILE*。
///
/// 语义取舍（POSIX 对"失败时原流状态"未作规定）：
/// - **先开新文件、再关旧的**：新文件打不开时原流**仍然可用**，不制造"流已被破坏"的中间态；
/// - `path == NULL`（POSIX 的"只改模式"用法）本实现**不支持** → EINVAL（如实声明）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freopen(
    path: *const c_char,
    mode: *const c_char,
    fp: *mut FILE,
) -> *mut FILE {
    if path.is_null() || mode.is_null() || fp.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let (fm, flags) = match parse_mode(mode) {
        Some(m) => m,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    let p = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    match libsys::open(p, flags, libsys::Permissions::read_write()) {
        Ok(newfd) => {
            let f = unsafe { &mut *fp };
            if !f.closed {
                // 同 fclose：必须经 libc 的 close 清掉该 fd 的位置表条目，否则新绑定的流会
                // 带着旧文件的位置（fd 号复用是常态）。
                let _ = crate::unistd::close(f.fd as c_int);
            }
            f.fd = newfd;
            f.mode = fm;
            f.eof = false;
            f.error = false;
            f.closed = false;
            f.buf_mode = FmBufMode::None;
            f.buf_ptr = core::ptr::null_mut();
            f.buf_len = 0;
            f.buf_pos = 0;
            f.pushback = -1;
            f.str_src = core::ptr::null();
            f.str_len = 0;
            f.str_pos = 0;
            fp
        }
        Err(e) => {
            set_errno(from_libsys(e));
            core::ptr::null_mut()
        }
    }
}

/// 解析 C 模式串 → `(FILE 模式, 打开旗标)`。**单点定义（S15）**。
///
/// 为什么两者必须**一起**返回：`"r+"` 与 `"w+"` 的 **FILE 模式相同**（都可读可写），
/// 但**打开旗标不同**（`w+` 要创建+截断，`r+` 绝不能截断）。分成两个函数解析必然漂移——
/// 本项首版正是分开写的，于是 `"w+"` 会拿到 `r+` 的旗标（不创建、不截断）——
/// 一个只在「文件不存在」时才显形的缺陷。
fn parse_mode(mode: *const c_char) -> Option<(FmMode, libsys::OpenFlags)> {
    unsafe {
        let m = mode as *const u8;
        let c0 = *m;
        let mut plus = false;
        let mut i = 0usize;
        while *m.add(i) != 0 {
            if *m.add(i) == b'+' {
                plus = true;
            }
            i += 1;
        }
        let append = libsys::OpenFlags {
            read: plus,
            write: true,
            create: true,
            truncate: false,
            append: true,
            directory: false,
            pipe: false,
            cloexec: false,
            exclusive: false,
        };
        match (c0, plus) {
            (b'r', false) => Some((FmMode::Read, libsys::OpenFlags::READ_ONLY)),
            (b'r', true) => Some((FmMode::ReadWrite, libsys::OpenFlags::READ_WRITE)),
            (b'w', false) => Some((FmMode::Write, libsys::OpenFlags::CREATE_OR_TRUNCATE)),
            (b'w', true) => Some((FmMode::ReadWrite, libsys::OpenFlags::CREATE_OR_TRUNCATE)),
            (b'a', false) => Some((FmMode::Append, append)),
            (b'a', true) => Some((FmMode::AppendRead, append)),
            _ => None,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE {
    if path.is_null() || mode.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let (fm, flags) = match parse_mode(mode) {
        Some(m) => m,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    let path_str = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    // 旗标与 FILE 分配都走单点（parse_mode / alloc_file，S15）。
    let perm = libsys::Permissions::read_write();
    match libsys::open(path_str, flags, perm) {
        Ok(fd) => {
            let fp = unsafe { alloc_file(fd, fm) };
            if fp.is_null() {
                // 分配失败：关掉刚打开的 fd，避免泄漏。
                let _ = crate::unistd::close(fd as c_int);
                return core::ptr::null_mut();
            }
            fp
        }
        Err(e) => {
            set_errno(from_libsys(e));
            core::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(fp: *mut FILE) -> c_int {
    if fp.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    unsafe {
        let f = &mut *fp;
        if f.closed {
            set_errno(EBADF);
            return EOF;
        }
        // **先冲刷再关**（有缓冲后这是数据不丢的唯一保证）。
        let _ = wflush(f);
        stream_unregister(fp);
        let fd = f.fd;
        f.closed = true;
        // tmpfile 的延迟删除：先删文件再关 fd（顺序无关，但先删更贴近 POSIX 语义）。
        tmp_release(fd as c_int);
        // **必须经 libc 的 close**（不是 libsys::close）：libc 的 close 会 `fd_pos_clear` 清掉
        // 用户态 fd 位置表里的条目。直接用 libsys::close 会**留下陈旧位置**——fd 号被复用时
        // 新流会带着上一个文件的位置，`fwrite` 于是走 `pwrite` 写到错误偏移。
        // 实测（tools/3psrc/libcc1 的对照实验）：tmpfile 读到 13 后关闭，同一 fd 号被
        // `fopen("w+")` 复用，写入落在偏移 13 ⇒ 文件大小 25（13+12）而开头 13 字节是 0。
        let _ = crate::unistd::close(fd as c_int);
        crate::malloc::free(fp as *mut u8);
    }
    0
}

/// FILE 层读写**单点**：所有流式 I/O 都经 libc 的 `read`/`write`，而不是直接调 `libsys`。
///
/// **为什么必须这样**（本轮实测的缺陷根因，不是风格问题）：`fseek`/`rewind` 把位置记在
/// **用户态的 fd 位置表**（libc/src/unistd.rs 的 `FD_POS`），而 libc 的 `read`/`write` 在
/// 该表已跟踪该 fd 时会改走**定位 I/O**（`pread`/`pwrite`）。此前 stdio 直接调
/// `libsys::read`/`libsys::write`（内核维护的**顺序**偏移）——**完全绕过那张表**，于是
/// `fseek(f, 0, SEEK_SET)` 之后再 `fread` 读到的仍是内核偏移处的数据：顺序写 12 字节后
/// 内核偏移已是 12 ⇒ 读到 **EOF（0 字节）**。实测普通文件与 `tmpfile` 都一样；
/// 对照实验（`tools/3psrc/libcc1`）把它与「unlink 之后写入不可见」明确区分开。
///
/// 统一走这里之后，FILE 流的位置语义与 `read`/`lseek` **同源**（S15）——
/// 也顺带让 `fseek` 之后 `fwrite` 真的从新位置写。
///
/// 返回 `isize`（同 POSIX `read`/`write`）：`< 0` 表示失败，**errno 已由 libc 的
/// `read`/`write` 设好**，故调用方**不得**再 `set_errno` 覆盖它。
#[inline]
fn fio_read(fd: u64, buf: &mut [u8]) -> isize {
    unsafe { crate::unistd::read(fd as c_int, buf.as_mut_ptr() as *mut c_void, buf.len()) }
}
/// **底层写调用计数**（诊断用，常驻、零成本：一次 Relaxed 自增）。
///
/// 存在的理由（S09 可观察）：判断"stdio 缓冲是否真的生效"**不能靠推测**——
/// 只要看这个计数就能判定"每次 fwrite 是否仍在做系统调用"。实测（2026-10）：
/// 2000 次 64 字节 fwrite 经 4 KiB 缓冲后**本应只有 ~32 次**底层写。
pub static FIO_WRITE_CALLS: AtomicUsize = AtomicUsize::new(0);
/// 底层写累计字节数（诊断用）。
pub static FIO_WRITE_BYTES: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn fio_write(fd: u64, buf: &[u8]) -> isize {
    FIO_WRITE_CALLS.fetch_add(1, Ordering::Relaxed);
    FIO_WRITE_BYTES.fetch_add(buf.len(), Ordering::Relaxed);
    unsafe { crate::unistd::write(fd as c_int, buf.as_ptr() as *const c_void, buf.len()) }
}

/// 读底层写计数（诊断 ABI，C 侧声明见 libc/include/boruix.h）。
#[unsafe(no_mangle)]
pub extern "C" fn boruix_stdio_write_calls() -> usize {
    FIO_WRITE_CALLS.load(Ordering::Relaxed)
}
/// 读底层写累计字节（诊断 ABI）。
#[unsafe(no_mangle)]
pub extern "C" fn boruix_stdio_write_bytes() -> usize {
    FIO_WRITE_BYTES.load(Ordering::Relaxed)
}

// ---------- 用户态缓冲层（2026-10 实测驱动的实现） ----------
//
// **为什么现在必须有**：此前本 libc 的 stdio **不做任何缓冲**，`fwrite`/`fputc`/`fputs`/
// `vfprintf` 每次调用直接把字节交给内核。而本内核的文件系统是**写直通**（`CachingByteDevice`
// 注释自陈"先落盘、成功后才更新缓存"），故每次 `write` 系统调用真的同步落盘一次——
// 机内实测 **8.9 毫秒/次**（正常系统 1~2 微秒）。
//
// 后果（全部有硬数据）：tcc 链接 `libc.a` 后节数 **4049**，写 ELF 时每节一次 `fwrite` ⇒
// 4049 × 8.9ms = **36 秒**（实测 "section headers" 35 秒）。整条链接 ~150 秒。
//
// **边界（必须与 `fflush`/`exit` 一起读）**：一旦有缓冲，"进程退出时缓冲里的数据"就成了
// 真实的数据丢失面。故本实现同时保证：`fflush` 获得**真实语义**（不再是空操作）、
// `fclose` 先冲刷再关、**`exit` 必须冲刷所有流**（`_exit` 按 POSIX 语义不冲刷）。

/// 缓冲容量（POSIX `BUFSIZ` 的常见取值）。
const WBUF_CAP: usize = 4096;
/// 同时打开的流上限（供 `fflush(NULL)` 与 `exit` 冲刷全部流）。
const MAX_OPEN_STREAMS: usize = 64;
static OPEN_STREAMS: [AtomicUsize; MAX_OPEN_STREAMS] =
    [const { AtomicUsize::new(0) }; MAX_OPEN_STREAMS];

/// 登记一个打开的流（`fopen`/`fdopen` 调用）。表满则如实放弃登记（返回 false）。
fn stream_register(fp: *mut FILE) -> bool {
    for s in OPEN_STREAMS.iter() {
        if s.load(Ordering::Relaxed) == 0
            && s.compare_exchange(0, fp as usize, Ordering::AcqRel, Ordering::Relaxed).is_ok()
        {
            return true;
        }
    }
    false
}
/// 注销一个流（`fclose` 调用）。
fn stream_unregister(fp: *mut FILE) {
    for s in OPEN_STREAMS.iter() {
        if s.load(Ordering::Relaxed) == fp as usize {
            s.store(0, Ordering::Release);
            return;
        }
    }
}

/// 惰性分配写缓冲。返回 false = 分配失败（**如实退回无缓冲**，绝不假装有缓冲）。
unsafe fn wbuf_ensure(f: &mut FILE) -> bool {
    unsafe {
        if !f.buf_ptr.is_null() {
            return true;
        }
        let p = crate::malloc::malloc(WBUF_CAP) as *mut u8;
        if p.is_null() {
            return false;
        }
        f.buf_ptr = p;
        f.buf_len = 0;
        f.buf_pos = 0;
        true
    }
}

/// 把写缓冲里的内容**全部**落盘（短写循环，如实失败）。返回 false = 出错（已置 `error`）。
unsafe fn wflush(f: &mut FILE) -> bool {
    unsafe {
        if f.buf_ptr.is_null() || f.buf_len == 0 {
            f.buf_len = 0;
            return true;
        }
        let mut done = 0usize;
        while done < f.buf_len {
            let slice = core::slice::from_raw_parts(f.buf_ptr.add(done), f.buf_len - done);
            let n = fio_write(f.fd, slice);
            if n <= 0 {
                f.error = true;
                f.buf_len = 0;
                return false;
            }
            done += n as usize;
        }
        f.buf_len = 0;
        true
    }
}

/// 把字节交给写缓冲（必要时先冲刷）。`line_flush` = 行缓冲模式下遇到 '\n' 时的立即冲刷。
unsafe fn wbuf_put(f: &mut FILE, bytes: &[u8], line_flush: bool) -> bool {
    unsafe {
        // 无缓冲（stderr / 分配失败退回 / 显式 None）：直接落盘，语义与改造前一致。
        if f.buf_mode == FmBufMode::None || !wbuf_ensure(f) {
            let mut done = 0usize;
            while done < bytes.len() {
                let n = fio_write(f.fd, &bytes[done..]);
                if n <= 0 { f.error = true; return false; }
                done += n as usize;
            }
            return true;
        }
        let mut off = 0usize;
        while off < bytes.len() {
            let space = WBUF_CAP - f.buf_len;
            if space == 0 {
                if !wflush(f) { return false; }
                continue;
            }
            let take = core::cmp::min(space, bytes.len() - off);
            core::ptr::copy_nonoverlapping(bytes.as_ptr().add(off), f.buf_ptr.add(f.buf_len), take);
            f.buf_len += take;
            off += take;
        }
        if line_flush && f.buf_mode == FmBufMode::Line && !wflush(f) {
            return false;
        }
        true
    }
}

/// 冲刷**所有**已登记的流（`fflush(NULL)` 与 `exit` 用）。返回 0 = 全部成功。
pub fn fflush_all() -> c_int {
    let mut rc = 0;
    for s in OPEN_STREAMS.iter() {
        let p = s.load(Ordering::Acquire) as *mut FILE;
        if !p.is_null() {
            unsafe {
                if !wflush(&mut *p) { rc = EOF; }
            }
        }
    }
    rc
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fread(ptr: *mut c_void, size: size_t, nmemb: size_t, fp: *mut FILE) -> size_t {
    if fp.is_null() || ptr.is_null() {
        set_errno(EINVAL);
        return 0;
    }
    let total = match size.checked_mul(nmemb) {
        Some(t) if t > 0 => t,
        _ => return 0,
    };
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_read() {
            set_errno(EINVAL);
            f.error = true;
            return 0;
        }
        let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, total);
        match fio_read(f.fd, buf) {
            n if n >= 0 => {
                let n = n as usize;
                if n < total {
                    f.eof = true;
                }
                n / size
            }
            _ => {
                // errno 已由 libc 的 read 设好，不覆盖。
                f.error = true;
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fwrite(ptr: *const c_void, size: size_t, nmemb: size_t, fp: *mut FILE) -> size_t {
    if fp.is_null() || ptr.is_null() {
        set_errno(EINVAL);
        return 0;
    }
    let total = match size.checked_mul(nmemb) {
        Some(t) if t > 0 => t,
        _ => return 0,
    };
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_write() {
            set_errno(EINVAL);
            f.error = true;
            return 0;
        }
        let buf = core::slice::from_raw_parts(ptr as *const u8, total);
        // 经用户态缓冲层（2026-10：见 wbuf_put 的说明——内核写直通 8.9ms/次，
        // 4049 次 fwrite 就是 36 秒）。无缓冲时 wbuf_put 内部直接落盘，语义不变。
        if wbuf_put(f, buf, false) {
            total / size
        } else {
            f.error = true;
            0
        }
    }
}

/// `fflush(fp)`：冲刷流的输出缓冲（POSIX）。
///
/// **本实现是刻意的空操作——不是未接线的桩**：本 libc 的 stdio **不做用户态缓冲**，
/// `fwrite`/`fputc`/`fputs`/`vfprintf` 每次调用都直接把字节交给内核（见 `fio_write` 的说明），
/// 因此**没有待冲刷的数据**，返回 0 就是事实。`fflush(NULL)`（冲刷全部流）同理。
///
/// **边界（S09，必须与缓冲层一起读）**：这条「空操作」与缓冲层是**同一个决定的两面**——
/// `setvbuf`/`setbuf` 之所以被判定为不支持（`docs/TODO/libc-posix-surface.md` 的 B 类），
/// 正是因为本 libc 没有缓冲层。一旦实现了缓冲层，`fflush` **必须同时获得真实语义**
/// （把缓冲写出去 + 复位游标），否则它就从「诚实的空操作」退化成**静默丢数据**。
///
/// 此前本函数没有文档注释，于是它在桩符号普查（`libc/tools/audit_stub_symbols.py`）里
/// 与真正的桩无法区分——「刻意的空操作」必须**写出来**才成立。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fflush(fp: *mut FILE) -> c_int {
    // **真实语义**（2026-10 起）：此前 stdio 无缓冲，空操作是诚实的；现在有缓冲层，
    // 空操作会变成**静默丢数据**，故必须真的把缓冲落盘。
    // fflush(NULL) = 冲刷所有已登记的流（POSIX）。
    if fp.is_null() {
        return fflush_all();
    }
    unsafe {
        let f = &mut *fp;
        if wflush(f) { 0 } else { EOF }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetc(fp: *mut FILE) -> c_int {
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_read() {
            f.error = true;
            set_errno(EINVAL);
            return EOF;
        }
        // pushback 槽优先。
        if f.pushback >= 0 {
            let c = f.pushback;
            f.pushback = -1;
            return c as c_int;
        }
        let mut b = [0u8; 1];
        match fio_read(f.fd, &mut b) {
            0 => { f.eof = true; EOF }
            n if n > 0 => b[0] as c_int,
            _ => {
                f.error = true;
                EOF
            }
        }
    }
}

/// `ungetc(c, fp)`：把字符 c 压回流（1 字节 pushback 槽）。
/// 成功返回 c，失败（流不可读 / 槽已满 / EOF 参数）返回 EOF。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ungetc(c: c_int, fp: *mut FILE) -> c_int {
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_read() {
            set_errno(EINVAL);
            return EOF;
        }
        if c == EOF {
            return EOF;
        }
        if f.pushback >= 0 {
            return EOF; // 槽已满（仅支持 1 字节 pushback）。
        }
        f.pushback = c & 0xFF;
        f.eof = false; // 压回后清除 EOF 标志。
        c & 0xFF
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputc(c: c_int, fp: *mut FILE) -> c_int {
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_write() {
            f.error = true;
            set_errno(EINVAL);
            return EOF;
        }
        let b = [(c & 0xFF) as u8; 1];
        // 行缓冲流遇换行符立即冲刷（保持交互可见性，同时仍把同行的多次写合并）。
        let line_flush = (c & 0xFF) as u8 == b'\n';
        if wbuf_put(f, &b, line_flush) {
            c & 0xFF
        } else {
            f.error = true;
            EOF
        }
    }
}


/// getc(fp)：等价于 fgetc(fp)。
///
/// **来路（3P6-2 第二波，真实报错驱动，不预猜）**：交叉构建 GMP（宿主 = Boruix）时
///   mpz/inp_str.c:58: error: call to undeclared function 'getc'
/// ——本 libc 此前**既没实现也没声明** getc/putc（属「整项缺失」；头文件覆盖审计只覆盖
/// 「已导出但未声明」，故没列出它们）。
///
/// POSIX 允许把 getc/putc 实现为宏；本实现提供**真函数**（取地址、当回调传递都可用），
/// 语义与 fgetc/fputc 完全一致（不另造缓冲语义）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getc(fp: *mut FILE) -> c_int {
    unsafe { fgetc(fp) }
}

/// putc(c, fp)：等价于 fputc(c, fp)。见 getc 的来路说明。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putc(c: c_int, fp: *mut FILE) -> c_int {
    unsafe { fputc(c, fp) }
}

/// fileno(fp)：返回 FILE 背后的文件描述符。
///
/// 来路（3P6-2 第二波「整项缺失」类，反向对账列出）：本 libc 的 FILE 结构本就带 fd，
/// 这个访问器只是把它暴露给 C。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fileno(fp: *mut FILE) -> c_int {
    unsafe {
        if fp.is_null() {
            set_errno(EINVAL);
            return -1;
        }
        (*fp).fd as c_int
    }
}

/// clearerr(fp)：清除 EOF 与错误标志（POSIX）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clearerr(fp: *mut FILE) {
    unsafe {
        if fp.is_null() {
            return;
        }
        (*fp).eof = false;
        (*fp).error = false;
    }
}

/// rewind(fp)：回到流开头并清除错误标志（POSIX）。
///
/// **为什么直接走 `lseek` 而不是本文件的 `fseek`**：两者等价（fseek 内部就是 lseek + 清
/// EOF/pushback），但 rewind 还要清**错误**标志，语义上是 clearerr + 复位。写成一条 lseek +
/// 一次 clearerr 比「调 fseek 再调 clearerr」少一层间接，也不依赖 fseek 的返回值。
/// （历史注记：本函数写下这段注释时 fseek/ftell 还是未接线的桩——那时用它们确实会把 rewind
/// 变成静默无效。fseek/ftell 已实现，本注释同步更新。）
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rewind(fp: *mut FILE) {
    unsafe {
        if fp.is_null() {
            return;
        }
        let _ = crate::unistd::lseek((*fp).fd as c_int, 0, 0 /* SEEK_SET */);
        clearerr(fp);
    }
}

/// perror(s)：把 `s: <errno 描述>` 打到 stderr（POSIX）。
///
/// **实现说明**：不用 fprintf（需要可变参数转发），而是分段落 fputs——输出与 POSIX 规定
/// 的格式一致（`s` 为空或 NULL 时只打描述）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn perror(s: *const c_char) {
    unsafe {
        let e = *crate::errno::__errno_location();
        let msg = crate::string::strerror(e);
        if !s.is_null() && *s != 0 {
            fputs(s, stderr);
            fputs(b": \0".as_ptr() as *const c_char, stderr);
        }
        if !msg.is_null() {
            fputs(msg, stderr);
        }
        fputs(b"\n\0".as_ptr() as *const c_char, stderr);
    }
}

/// \`fgets(s, n, fp)\`：从 fp 读取至多 n-1 字符，遇换行或 EOF 停止，结果 NUL 终止。
/// 返回 s；读到 EOF 且无字符时返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgets(s: *mut c_char, n: c_int, fp: *mut FILE) -> *mut c_char {
    if s.is_null() || n <= 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let max = (n - 1) as usize;
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_read() {
            f.error = true;
            set_errno(EINVAL);
            return core::ptr::null_mut();
        }
        let mut i = 0usize;
        while i < max {
            // pushback 槽优先。
            if f.pushback >= 0 {
                let c = f.pushback;
                f.pushback = -1;
                *s.add(i) = c as c_char;
                i += 1;
                if c == b'\n' as i32 { break; }
                continue;
            }
            let mut b = [0u8; 1];
            match fio_read(f.fd, &mut b) {
                0 => {
                    f.eof = true;
                    break;
                }
                n if n > 0 => {
                    *s.add(i) = b[0] as c_char;
                    i += 1;
                    if b[0] == b'\n' {
                        break;
                    }
                }
                _ => {
                    f.error = true;
                    return core::ptr::null_mut();
                }
            }
        }
        if i == 0 {
            // 未读到任何字符（EOF 或错误已置标志）。
            if f.eof || f.error {
                return core::ptr::null_mut();
            }
        }
        *s.add(i) = 0;
        s
    }
}


/// `getdelim(lineptr, n, delim, fp)`：按分隔符读取一行（自动扩容）。
/// 返回读取的字符数（不含分隔符），EOF/错误返回 -1。*lineptr 须指向可释放缓冲区或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getdelim(
    lineptr: *mut *mut c_char,
    n: *mut size_t,
    delim: c_int,
    fp: *mut FILE,
) -> isize {
    unsafe {
        if lineptr.is_null() || n.is_null() || fp.is_null() {
            set_errno(EINVAL);
            return -1;
        }
        let f = &mut *fp;
        if !f.mode.can_read() {
            set_errno(EINVAL);
            return -1;
        }
        if (*lineptr).is_null() {
            let cap: size_t = 128;
            let buf = crate::malloc::malloc(cap);
            if buf.is_null() {
                set_errno(crate::errno::ENOMEM);
                return -1;
            }
            *lineptr = buf as *mut c_char;
            *n = cap;
        }
        let mut ptr = *lineptr;
        let mut cap = *n;
        let mut len: size_t = 0;
        let dl = (delim & 0xFF) as u8;
        let mut total: isize = 0;
        loop {
            let c = fscan_getc(f);
            if c < 0 {
                break;
            }
            if len + 1 >= cap {
                let newcap = cap * 2;
                let nb = crate::malloc::realloc(ptr as *mut u8, newcap);
                if nb.is_null() {
                    set_errno(crate::errno::ENOMEM);
                    return -1;
                }
                ptr = nb as *mut c_char;
                *lineptr = ptr;
                *n = newcap;
                cap = newcap;
            }
            *ptr.add(len) = c as c_char;
            len += 1;
            total += 1;
            if c as u8 == dl {
                break;
            }
        }
        if total == 0 {
            if f.eof || f.error {
                return -1;
            }
            return -1;
        }
        *ptr.add(len) = 0;
        total
    }
}

/// `getline(lineptr, n, fp)`：按换行读取一行（等价 getdelim(delim='\n')）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getline(
    lineptr: *mut *mut c_char,
    n: *mut size_t,
    fp: *mut FILE,
) -> isize {
    unsafe { getdelim(lineptr, n, b'\n' as c_int, fp) }
}


/// \`fputs(s, fp)\`：写字符串到 fp（不含 NUL）。成功返回非负，失败 EOF。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputs(s: *const c_char, fp: *mut FILE) -> c_int {
    if s.is_null() || fp.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    unsafe {
        let f = &mut *fp;
        if !f.mode.can_write() {
            f.error = true;
            set_errno(EINVAL);
            return EOF;
        }
        let mut len = 0usize;
        while *s.add(len) != 0 {
            len += 1;
        }
        if len == 0 {
            return 0;
        }
        let buf = core::slice::from_raw_parts(s as *const u8, len);
        if wbuf_put(f, buf, false) {
            return 0;
        }
        match fio_write(f.fd, buf) {
            n if n >= 0 => 0,
            _ => {
                f.error = true;
                EOF
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn feof(fp: *mut FILE) -> c_int {
    unsafe { (*fp).eof as c_int }
}

#[unsafe(no_mangle)]
pub extern "C" fn ferror(fp: *mut FILE) -> c_int {
    unsafe { (*fp).error as c_int }
}

/// `fseek(fp, offset, whence)`：重定位流（POSIX）。成功返回 0，失败 -1 置 errno。
///
/// **此前是未接线的桩**（忽略参数直接 ENOTSUP）——桩的符号存在，故反向对账把它算作
/// 「已实现」，**证明不了行为对**。这正是 `tools/3psrc/libcc1` 那类系统内运行时验收存在的
/// 理由：它在系统内真调一次 fseek，桩立刻现形。
///
/// 语义要点：
///  - 清除流的 EOF 标志并**丢弃 `ungetc` 的 pushback**（POSIX 明确要求；不清会把回退的
///    字节留在后续读里，读出的内容与文件不符）；
///  - 真正的定位交给 `lseek`（**单点定义**：`fd` 定位逻辑只有那一份，S15）。`lseek` 一旦
///    被调用，该 fd 就转入「用户态维护位置」模式，后续 `read`/`write` 走定位 I/O——
///    这正是 fseek 之后 `fread` 能读回开头数据的机制。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fseek(fp: *mut FILE, offset: c_long, whence: c_int) -> c_int {
    if fp.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let f = &mut *fp;
    f.eof = false;
    f.pushback = -1;
    // 错误标志按 POSIX **不**由 fseek 清除（那是 clearerr/rewind 的事）。
    if crate::unistd::lseek(f.fd as c_int, offset, whence) < 0 {
        return -1;
    }
    0
}

/// `ftell(fp)`：返回当前流位置（POSIX）。失败返回 -1 置 errno。
///
/// 实现是 `lseek(fd, 0, SEEK_CUR)`。**诚实边界**：若该 fd 从未被定位过，`lseek` 的 SEEK_CUR
/// 分支会如实返回 `ENOTSUP`（用户态不知道内核维护的当前位置，且内核不暴露它）——
/// 不猜、不返回 0 冒充。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftell(fp: *mut FILE) -> c_long {
    if fp.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    crate::unistd::lseek((*fp).fd as c_int, 0, 1 /* SEEK_CUR */)
}

// ---------- 内部辅助 ----------

/// 把 C 字符串取为字节切片（直到 NUL）。**零分配。**
///
/// # 为什么不再复制成 `Vec`
///
/// 旧实现 `cstr_to_bytes` 每次调用都分配一块 Rust `Vec`。`printf`/`puts`/`fputs` 等
/// **每次调用**都会走这里，于是 Rust 侧的 buddy 全局分配器与 C 侧的 `malloc` **交替**
/// 推进同一个 `brk`——实测该交替会破坏 buddy 的 free_list（见
/// `tcc-on-boruix/boruix/CRT-AND-LIBS`）。改为借用后，这些路径完全不再分配。
///
/// # Safety
///
/// `p` 必须指向以 NUL 结尾、且在返回的切片被使用期间保持有效的 C 字符串。
pub unsafe fn cstr_bytes<'a>(p: *const c_char) -> &'a [u8] {
    unsafe {
        let mut n = 0usize;
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p as *const u8, n)
    }
}

/// 把 C 字符串转换为 Rust `&str`（用于 libsys 的 &str 参数）。**零分配。**
///
/// 直接在 C 字符串本身上取切片（长度 = 到 NUL 为止），不复制、不分配。
/// 非法 UTF-8 返回 None（调用方置 EINVAL，S02 显式处理编码）。
///
/// # 为什么不再是「复制进 Vec 再 Box::leak」
///
/// 旧实现把字节复制进 Rust `Vec` 再 `Box::leak` 成 `'static`——那会让**每一次**路径转换
/// 都**泄漏**一块 Rust 堆内存，并且让 C 侧的 `malloc` 与 Rust 侧的 buddy 全局分配器
/// **交替**推进同一个 `brk`。实测该交替会破坏 buddy 的 free_list（见
/// `tcc-on-boruix/boruix/CRT-AND-LIBS`）。零分配同时消掉泄漏与交替。
///
/// # Safety
///
/// `p` 必须指向以 NUL 结尾、且在返回的 `&str` 被使用期间保持有效的 C 字符串。
/// 所有现有调用点都是「取到后立即用于 libsys 调用」，满足该约定。
pub unsafe fn cstr_to_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe {
        let mut n = 0usize;
        while *p.add(n) != 0 {
            n += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(p as *const u8, n)).ok()
    }
}

/// 内部：把格式化结果写到内存缓冲的 sink（sprintf/snprintf 用）。
struct MemSink<'a> {
    buf: &'a mut [u8],
    pos: usize,
    truncated: bool,
}
impl FmtSink for MemSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        for &b in bytes {
            if self.pos < self.buf.len() {
                self.buf[self.pos] = b;
                self.pos += 1;
            } else {
                self.truncated = true;
            }
        }
        Ok(())
    }
    fn write_byte(&mut self, b: u8) -> Result<(), ()> {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        } else {
            self.truncated = true;
        }
        Ok(())
    }
    fn count(&self) -> usize {
        self.pos
    }
}
// ---------- printf 核心（经 VaList 读取可变参数） ----------

use core::ffi::VaList;
/// 从 va_list 的 **GP 寄存器区** 读取下一个变长实参，返回其 u64 位型。
///
/// 本目标 `x86_64-unknown-none` 的 `c_variadic`（nightly 特性）在**调用侧**的代码生成
/// 存在 ABI 缺陷：变长 `double` 实参被放进通用寄存器（GPR）而非 XMM，且 `%al=0`（声明
/// 未用向量寄存器）。因此标准的 `ap.next_arg::<f64>()`（走 `fp_offset`/XMM 槽）读到的是
/// 从未被 spill 的垃圾值。实测（QEMU 真机）确认调用方把**全部**变长实参（整型/指针/浮点）
/// 按序放入 GPR，故统一经 `gp_offset`/`reg_save_area` 读取可正确还原（整型/指针本就走此路径；
/// 浮点改走此路径即修复）。这是对编译器缺陷的显式规避（详见 shell/README 已知限制章节）。
///
/// 调用后 `gp_offset` 前进 8（每个变长实参占一个 8 字节 GP 槽）。
/// 从 va_list 读取下一个变长 `double`。**ABI 相关，必须按目标分派**：
///
/// - `os="boruix"`（用户态目标，硬浮点 +SSE）：走标准的 FP 槽；
/// - `os="none"`（内核目标，`rustc-abi: softfloat`）：该 ABI 下变长 `double` 由调用方放进
///   **通用寄存器**，只能走 GP 槽。
///
/// 此前这里只实现了后者，并把它描述成「编译器 c_variadic 缺陷」——**根因其实是目标 ABI 是
/// soft-float**（3P3-2 实测：用户态改用硬浮点目标后，只有标准路径才读得对；`%f` 一度全错）。
#[cfg(target_os = "boruix")]
unsafe fn next_float_arg(ap: &mut VaList) -> f64 {
    unsafe { ap.next_arg::<f64>() }
}

#[cfg(not(target_os = "boruix"))]
unsafe fn next_float_arg(ap: &mut VaList) -> f64 {
    f64::from_bits(unsafe { next_float_arg_gp(ap) })
}

#[cfg(not(target_os = "boruix"))]
unsafe fn next_float_arg_gp(ap: &mut VaList) -> u64 {
    #[repr(C)]
    struct VL {
        gp_offset: i32,
        fp_offset: i32,
        overflow_arg_area: *const u8,
        reg_save_area: *const u8,
    }
    let vl: *mut VL = unsafe { core::mem::transmute(ap as *mut VaList as *mut VL) };
    let gp = unsafe { (*vl).gp_offset } as usize;
    let area = unsafe { (*vl).reg_save_area };
    let v = unsafe { core::ptr::read(area.add(gp) as *const u64) };
    unsafe { (*vl).gp_offset += 8 };
    v
}

/// 从一个 VaList 读取 width/precision 的 \`*\` 占位。
fn resolve_star(spec: &Spec, ap: &mut VaList) -> (i64, i64) {
    let mut width = spec.width;
    if width == -2 {
        let w = unsafe { ap.next_arg::<i32>() } as i64;
        width = if w < 0 { -w } else { w };
    }
    let mut prec = spec.prec;
    if prec == -2 {
        let p = unsafe { ap.next_arg::<i32>() } as i64;
        prec = if p < 0 { -1 } else { p };
    }
    (width, prec)
}

/// 渲染单个说明符，从 ap 读取参数写入 sink。
fn render_spec(
    spec: &Spec,
    emitted_before: usize,
    ap: &mut VaList,
    sink: &mut dyn FmtSink,
) -> Result<(), ()> {
    let (width, prec) = resolve_star(spec, ap);
    let mut s = *spec;
    s.width = width;
    s.prec = prec;

    match s.conv {
        Conv::Percent => sink.write_byte(b'%'),
        Conv::Count => {
            let p = unsafe { ap.next_arg::<*mut i32>() };
            if !p.is_null() {
                unsafe { *p = emitted_before as i32; }
            }
            Ok(())
        }
        Conv::Char => {
            let c = unsafe { ap.next_arg::<i32>() } as u8;
            emit_char(&s, c, sink)
        }
        Conv::Str => {
            let p = unsafe { ap.next_arg::<*const c_char>() };
            if p.is_null() {
                emit_str(&s, b"(null)", sink)
            } else {
                let bytes = unsafe { cstr_bytes(p) };
                emit_str(&s, &bytes, sink)
            }
        }
        Conv::Ptr => {
            let p = unsafe { ap.next_arg::<*const c_void>() };
            let v = p as usize as u64;
            let mut ps = s;
            ps.conv = Conv::Hex;
            ps.alt = true;
            ps.len = Length::Ll;
            emit_int(&ps, v, false, sink)
        }
        Conv::Int => {
            let raw = read_signed_arg(&s, ap) as i64;
            let neg = raw < 0;
            let abs = if neg { raw.wrapping_neg() as u64 } else { raw as u64 };
            emit_int(&s, abs, neg, sink)
        }
        Conv::UInt | Conv::Oct | Conv::Hex | Conv::Bin => {
            let v = read_unsigned_arg(&s, ap);
            emit_int(&s, v, false, sink)
        }
        Conv::Float | Conv::Exp | Conv::General => {
            let v = unsafe { next_float_arg(ap) };
            let precision = if s.prec >= 0 { s.prec as usize } else { 6 };
            let mut d = crate::float::decompose(v);
            match s.conv {
                Conv::Float => crate::float::emit_fixed(&s, &mut d, precision, sink),
                Conv::Exp => crate::float::emit_exp(&s, &mut d, precision, sink),
                _ => crate::float::emit_general(&s, &mut d, precision, sink),
            }
        }
        Conv::HexFloat => {
            let v = unsafe { next_float_arg(ap) };
            crate::float::emit_hexfloat(&s, v, sink)
        }
    }
}

/// 按 length 读取有符号整数参数。
fn read_signed_arg(spec: &Spec, ap: &mut VaList) -> u64 {
    match spec.len {
        Length::None | Length::Hh | Length::H => unsafe { ap.next_arg::<i32>() as i64 as u64 },
        _ => unsafe { ap.next_arg::<i64>() as u64 },
    }
}

/// 按 length 读取无符号整数参数。
fn read_unsigned_arg(spec: &Spec, ap: &mut VaList) -> u64 {
    match spec.len {
        Length::None | Length::Hh | Length::H => unsafe { ap.next_arg::<u32>() as u64 },
        _ => unsafe { ap.next_arg::<u64>() },
    }
}

/// 核心：格式化 fmt 到 sink，经 VaList 读参数。
fn vformat_to_sink(
    fmt: *const c_char,
    ap: &mut VaList,
    sink: &mut dyn FmtSink,
) -> Result<ssize_t, ()> {
    let fmt_bytes = unsafe { cstr_bytes(fmt) };
    parse_and_format(&fmt_bytes, sink, |spec, sink| {
        let before = sink.count();
        render_spec(spec, before, ap, sink)?;
        Ok(())
    })?;
    Ok(sink.count() as ssize_t)
}

/// 核心：格式化到内存缓冲（sprintf/snprintf），返回应写字节数。
fn vformat_mem(
    fmt: *const c_char,
    ap: &mut VaList,
    buf: *mut u8,
    cap: usize,
) -> Result<ssize_t, ()> {
    // `sprintf` 语义上无上界（由调用方保证缓冲区足够大），它经 `usize::MAX` 传入。
    //
    // **但 `slice::from_raw_parts_mut` 的契约要求 `len <= isize::MAX`**：传 `usize::MAX`
    // 是 UB，而且会让 `MemSink::write` 的边界检查（`self.pos < self.buf.len()`）形同虚设，
    // 于是写出缓冲区一路写到代码段才崩（实测：tcc 的 sprintf 就是这样崩的，
    // 内核留证 fault_addr 落在 emit_str 的代码里、err=0x7 用户态写）。
    //
    // 收敛到 `isize::MAX`：满足切片契约，同时保持「实际上无界」的语义。
    let cap = cap.min(isize::MAX as usize);
    if cap == 0 || buf.is_null() {
        // 只统计长度（snprintf cap=0 合法）。
        let fmt_bytes = unsafe { cstr_bytes(fmt) };
        let mut counter = CounterSink { n: 0 };
        parse_and_format(&fmt_bytes, &mut counter, |spec, sink| {
            let before = sink.count();
            render_spec(spec, before, ap, sink)?;
            Ok(())
        })?;
        return Ok(counter.n as ssize_t);
    }
    let mut mem = MemSink {
        buf: unsafe { core::slice::from_raw_parts_mut(buf, cap) },
        pos: 0,
        truncated: false,
    };
    let fmt_bytes = unsafe { cstr_bytes(fmt) };
    parse_and_format(&fmt_bytes, &mut mem, |spec, sink| {
        let before = sink.count();
        render_spec(spec, before, ap, sink)?;
        Ok(())
    })?;
    // NUL 终止。
    if mem.pos < cap {
        unsafe { *buf.add(mem.pos) = 0; }
    }
    Ok(mem.pos as ssize_t)
}

/// 只计数不输出的 sink。
struct CounterSink { n: usize }
impl FmtSink for CounterSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> { self.n += bytes.len(); Ok(()) }
    fn write_byte(&mut self, _b: u8) -> Result<(), ()> { self.n += 1; Ok(()) }
    fn count(&self) -> usize { self.n }
}

/// 输出到 fd 的 sink。
struct FdSink {
    fd: u64,
    wrote: ssize_t,
}
impl FmtSink for FdSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let mut chunk = [0u8; 256];
        let mut i = 0;
        for &b in bytes {
            chunk[i] = b;
            i += 1;
            if i == chunk.len() {
                self.flush(&chunk[..i]);
                i = 0;
            }
        }
        if i > 0 {
            self.flush(&chunk[..i]);
        }
        Ok(())
    }
    fn write_byte(&mut self, b: u8) -> Result<(), ()> {
        self.write(core::slice::from_ref(&b))
    }
    fn count(&self) -> usize { self.wrote as usize }
}
impl FdSink {
    fn flush(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // 经 FILE 层的同一条 I/O 单点（printf 家族也必须认 fd 位置表）。
        let n = fio_write(self.fd, bytes);
        if n >= 0 {
            self.wrote += n as ssize_t;
        }
    }
}

// ---------- 公开 c_variadic 入口 ----------

/// \`printf(fmt, ...)\`：格式化到 stdout（fd 1）。返回写出字符数或 -1。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn printf(fmt: *const c_char, ap: ...) -> c_int {
    stdio_init();
    vprintf(fmt, unsafe { core::mem::transmute::<_, VaList>(ap) })
}

/// \`vprintf(fmt, ap)\`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vprintf(fmt: *const c_char, ap: VaList<'_>) -> c_int {
    stdio_init();
    if fmt.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut sink = FdSink { fd: 1, wrote: 0 };
    let mut ap = ap;
    match vformat_to_sink(fmt, &mut ap, &mut sink) {
        Ok(n) => n as c_int,
        Err(_) => -1,
    }
}

/// \`fprintf(fp, fmt, ...)\`：格式化到 FILE。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fprintf(fp: *mut FILE, fmt: *const c_char, ap: ...) -> c_int {
    vfprintf(fp, fmt, unsafe { core::mem::transmute::<_, VaList>(ap) })
}

/// \`vfprintf(fp, fmt, ap)\`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfprintf(fp: *mut FILE, fmt: *const c_char, ap: VaList<'_>) -> c_int {
    stdio_init();
    if fp.is_null() || fmt.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let f = unsafe { &mut *fp };
    if !f.mode.can_write() {
        set_errno(EINVAL);
        return -1;
    }
    let fd = f.fd;
    let mut sink = FdSink { fd, wrote: 0 };
    let mut ap = ap;
    match vformat_to_sink(fmt, &mut ap, &mut sink) {
        Ok(n) => n as c_int,
        Err(_) => -1,
    }
}

/// \`sprintf(buf, fmt, ...)\`：格式化到字符串。返回写入字符数（不含 NUL）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sprintf(buf: *mut c_char, fmt: *const c_char, ap: ...) -> c_int {
    vsnprintf(buf, usize::MAX, fmt, unsafe { core::mem::transmute::<_, VaList>(ap) }) as c_int
}

/// vsprintf(buf, fmt, ap)：格式化到字符串（**无长度上限**，由调用方保证缓冲区足够大），
/// 参数来自 VaList。
///
/// **来路（3P6-2 第二波，真实报错驱动，不预猜）**：交叉构建 GMP 时
///   printf/sprintffuns.c:56:3: error: call to undeclared function 'vsprintf'
/// ——<stdio.h> 有 vsnprintf/sprintf 但没有 vsprintf。
///
/// **实现说明（为什么这样写才对）**：无界语义靠 `vsnprintf(.., usize::MAX, ..)`——
/// `vformat_mem` 会把 cap 收敛到 `isize::MAX`（满足切片契约，同时实际无界），
/// 这正是 `sprintf` 走的路。**不**另造一个「很大的 cap」假装无界（那是静默截断，S09 不允许）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsprintf(buf: *mut c_char, fmt: *const c_char, ap: VaList<'_>) -> c_int {
    unsafe { vsnprintf(buf, usize::MAX, fmt, ap) }
}

/// \`snprintf(buf, size, fmt, ...)\`：格式化到有界字符串，恒 NUL 终止（size>0）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn snprintf(buf: *mut c_char, size: size_t, fmt: *const c_char, ap: ...) -> c_int {
    vsnprintf(buf, size, fmt, unsafe { core::mem::transmute::<_, VaList>(ap) }) as c_int
}

/// \`vsnprintf(buf, size, fmt, ap)\`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsnprintf(buf: *mut c_char, size: size_t, fmt: *const c_char, ap: VaList<'_>) -> c_int {
    stdio_init();
    if fmt.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut ap = ap;
    if size == 0 {
        // 只返回应写长度。
        let fmt_bytes = unsafe { cstr_bytes(fmt) };
        let mut counter = CounterSink { n: 0 };
        let mut emitted = 0usize;
        match parse_and_format(&fmt_bytes, &mut counter, |spec, sink| {
            let before = emitted;
            render_spec(spec, before, &mut ap, sink)?;
            emitted += 1;
            Ok(())
        }) {
            Ok(_) => counter.n as c_int,
            Err(_) => -1,
        }
    } else {
        match vformat_mem(fmt, &mut ap, buf as *mut u8, size) {
            Ok(n) => n as c_int,
            Err(_) => -1,
        }
    }
}

/// \`puts(s)\`：输出字符串 + 换行。成功返回非负，失败 EOF。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(s: *const c_char) -> c_int {
    stdio_init();
    if s.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    let bytes = unsafe { cstr_bytes(s) };
    let mut sink = FdSink { fd: 1, wrote: 0 };
    sink.write(&bytes).ok();
    sink.write(b"\n").ok();
    if sink.wrote >= 0 {
        sink.wrote as c_int
    } else {
        EOF
    }
}

/// \`putchar(c)\`：输出单字符到 stdout。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putchar(c: c_int) -> c_int {
    stdio_init();
    let b = [(c & 0xFF) as u8; 1];
    match fio_write(1, &b) {
        n if n >= 0 => c & 0xFF,
        _ => {
            EOF
        }
    }
}

/// \`getchar()\`：从 stdin 读一字符；EOF 或错误返回 EOF。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getchar() -> c_int {
    stdio_init();
    let mut b = [0u8; 1];
    match fio_read(0, &mut b) {
        0 => EOF,
        n if n > 0 => b[0] as c_int,
        _ => {
            EOF
        }
    }
}
// ---------- 格式化输入（fscanf 最小子集） ----------

/// 内部：从 FILE 读一个字节；EOF 返回 -1。
unsafe fn fscan_getc(f: &mut FILE) -> i32 {
    // pushback 槽优先——**与源类型无关**（内存源也要能回退，否则 sscanf 的
    // 「读超了再吐回一个字符」会失效）。
    if f.pushback >= 0 {
        let c = f.pushback;
        f.pushback = -1;
        return c;
    }
    // 内存源（sscanf）：读完即 EOF。
    if !f.str_src.is_null() {
        if f.str_pos >= f.str_len {
            f.eof = true;
            return -1;
        }
        let c = unsafe { *f.str_src.add(f.str_pos) };
        f.str_pos += 1;
        return c as i32;
    }
    let mut b = [0u8; 1];
    match fio_read(f.fd, &mut b) {
        0 => { f.eof = true; -1 }
        n if n > 0 => b[0] as i32,
        _ => { f.error = true; -1 }
    }
}

/// 把一个字符压回流（1 字节 pushback 槽；若已满则忽略，返回 false）。
unsafe fn fscan_ungetc(f: &mut FILE, c: i32) -> bool {
    if c < 0 {
        return false;
    }
    if f.pushback >= 0 {
        return false; // 槽已满，不支持多字节 pushback。
    }
    f.pushback = c;
    true
}

fn fscan_isspace(c: i32) -> bool {
    c == b' ' as i32 || c == b'\t' as i32 || c == b'\n' as i32
        || c == b'\r' as i32 || c == b'\x0b' as i32 || c == b'\x0c' as i32
}

/// 跳过前导空白，返回首个非空白字符（EOF 为负）。
fn fscan_first_non_white(f: &mut FILE) -> i32 {
    let mut c = unsafe { fscan_getc(f) };
    while c >= 0 && fscan_isspace(c) {
        c = unsafe { fscan_getc(f) };
    }
    c
}

/// 长度修饰符（fscanf 目标指针宽度）。
#[derive(Clone, Copy, PartialEq)]
enum FScanLen {
    None,
    Hh,
    H,
    L,
    Ll,
    Z,
    T,
    BigL,
}

/// 把有符号 value 写入宽度由 len 决定的目标整数指针（i8/i16/i32/i64）。
fn write_int_width(p: *mut u8, value: i64, len: &FScanLen) {
    unsafe {
        match len {
            FScanLen::Hh => *(p as *mut i8) = value as i8,
            FScanLen::H => *(p as *mut i16) = value as i16,
            FScanLen::Ll | FScanLen::Z | FScanLen::T | FScanLen::BigL => {
                *(p as *mut i64) = value
            }
            _ => *(p as *mut i32) = value as i32,
        }
    }
}

/// 把无符号 value 写入宽度由 len 决定的目标整数指针（u8/u16/u32/u64）。
fn write_uint_width(p: *mut u8, value: u64, len: &FScanLen) {
    unsafe {
        match len {
            FScanLen::Hh => *(p as *mut u8) = value as u8,
            FScanLen::H => *(p as *mut u16) = value as u16,
            FScanLen::Ll | FScanLen::Z | FScanLen::T | FScanLen::BigL => {
                *(p as *mut u64) = value
            }
            _ => *(p as *mut u32) = value as u32,
        }
    }
}

/// 10^k（整数 k，范围受限时退化到边界；用于 fscanf 实数组装）。
/// 手动实现以在 no_std 下避开 \`f64::powi\` 的方法可用性问题。
pub fn f64_pow10(k: i32) -> f64 {
    // 预计算 1e0..1e308 与 1e-1..1e-308 的对数表太占内存；用连乘，够用。
    let mut r = 1.0f64;
    let mut n = k;
    if n >= 0 {
        while n > 0 {
            r *= 10.0;
            n -= 1;
        }
    } else {
        while n < 0 {
            r /= 10.0;
            n += 1;
        }
    }
    r
}

/// \`fscanf(fp, fmt, ...)\`：从 fp 读取格式化输入。
///
/// 支持转换：\`%d %i %u %x %o %s %c %f %e %g %n %% %[\`（含大写 \`%F/%E/%G\`），
/// 长度修饰符 \`hh h l ll z t L j\`，可选宽度（如 \`%5d\`）与抑制赋值
/// （\`%*d\`）。整数目标指针宽度随长度修饰符变化；实数 \`%f→float*\`、
/// \`%lf→double*\`、\`%Lf→long double*\`（本目标 long double≈f64，如实）。
/// **pushback（S19）**：FILE 带 1 字节 pushback 槽（ungetc），fscanf 读到不匹配
/// 字符（非本转换数字/集合外/超宽/实数终止符）时压回供后续转换复用，避免字符丢失；
/// 多字节 pushback 仅支持 1 槽。%lc/%ls（l 修饰宽字符）目标为 wchar_t*（x86_64 上 4 字节），
/// 每字节扩展为宽字符。返回成功赋值项数；遇 EOF/错误返回 EOF。
/// `fscanf(fp, fmt, ...)`：从流读取格式化输入。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fscanf(fp: *mut FILE, fmt: *const c_char, ap: ...) -> c_int {
    use core::ffi::VaList;
    if fp.is_null() || fmt.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    let ap: VaList = unsafe { core::mem::transmute(ap) };
    unsafe { vscan(&mut *fp, fmt, ap) }
}

/// `sscanf(s, fmt, ...)`：从**字符串**读取格式化输入。
///
/// 与 `fscanf` 共用同一套扫描核心 `vscan`（解析逻辑单点定义，S15）：本函数只是把
/// 输入源换成内存缓冲区。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sscanf(s: *const c_char, fmt: *const c_char, ap: ...) -> c_int {
    use core::ffi::VaList;
    if s.is_null() || fmt.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    let bytes = unsafe { crate::stdio::cstr_bytes(s) };
    let mut f = FILE {
        fd: u64::MAX, // 无 fd：本 FILE 是纯内存源（不参与 fd 路径）
        mode: FmMode::Read,
        buf_mode: FmBufMode::None,
        eof: false,
        error: false,
        buf_ptr: core::ptr::null_mut(),
        buf_len: 0,
        buf_pos: 0,
        closed: false,
        pushback: -1,
        str_src: bytes.as_ptr(),
        str_len: bytes.len(),
        str_pos: 0,
    };
    let ap: VaList = unsafe { core::mem::transmute(ap) };
    unsafe { vscan(&mut f, fmt, ap) }
}

/// 格式化扫描的**共同核心**：`fscanf` 与 `sscanf` 都走这里。
///
/// `f` 的输入源由 `FILE::str_src` 决定（非 null 即内存源），故本函数与源类型无关。
unsafe fn vscan(f: &mut FILE, fmt: *const c_char, mut ap: VaList) -> c_int {
    if !f.mode.can_read() {
        f.error = true;
        set_errno(EINVAL);
        return EOF;
    }
    let fmt_bytes = crate::stdio::cstr_bytes(fmt);
        let mut i = 0usize;
        let mut assigned: c_int = 0;
        // 从流累计消费的字符数（用于 %n）。
        let mut consumed: isize = 0;

        // 长度修饰符解析（映射到模块级 FScanLen，供宽度写入使用）。
        let use_len = |c: u8| match c {
            b'h' => Some(FScanLen::Hh),
            b'l' => Some(FScanLen::L),
            b'z' => Some(FScanLen::Z),
            b't' => Some(FScanLen::T),
            b'L' => Some(FScanLen::BigL),
            b'j' => Some(FScanLen::Ll),
            _ => None,
        };

        loop {
            if i >= fmt_bytes.len() {
                break;
            }
            let c = fmt_bytes[i];
            if c == b'%' {
                i += 1;
                // %% 字面量。
                if i < fmt_bytes.len() && fmt_bytes[i] == b'%' {
                    if fscan_getc(f) == b'%' as i32 {
                        consumed += 1;
                    } else {
                        break;
                    }
                    i += 1;
                    continue;
                }
                let mut suppress = false;
                if i < fmt_bytes.len() && fmt_bytes[i] == b'*' {
                    suppress = true;
                    i += 1;
                }
                let mut width: usize = 0;
                while i < fmt_bytes.len() && fmt_bytes[i].is_ascii_digit() {
                    width = width * 10 + (fmt_bytes[i] - b'0') as usize;
                    i += 1;
                }
                // 长度修饰符（支持 hh/l/ll/z/t/L/j）。
                let mut len = FScanLen::None;
                if i < fmt_bytes.len() {
                    if fmt_bytes[i] == b'h' {
                        if i + 1 < fmt_bytes.len() && fmt_bytes[i + 1] == b'h' {
                            len = FScanLen::Hh;
                            i += 2;
                        } else {
                            len = FScanLen::H;
                            i += 1;
                        }
                    } else if fmt_bytes[i] == b'l' {
                        if i + 1 < fmt_bytes.len() && fmt_bytes[i + 1] == b'l' {
                            len = FScanLen::Ll;
                            i += 2;
                        } else {
                            len = FScanLen::L;
                            i += 1;
                        }
                    } else if let Some(l) = use_len(fmt_bytes[i]) {
                        len = l;
                        i += 1;
                    }
                }
                if i >= fmt_bytes.len() {
                    break;
                }
                let conv = fmt_bytes[i];
                i += 1;
                let w = if width == 0 { usize::MAX } else { width };

                // %n：把已消费字符数写入整数指针（长度修饰决定宽度）。
                if conv == b'n' {
                    if !suppress {
                        let p = ap.next_arg::<usize>() as *mut u8;
                        if !p.is_null() {
                            write_int_width(p, consumed as i64, &len);
                            assigned += 1;
                        }
                    }
                    continue;
                }

                // %c / %lc：读 width（默认 1）个原样字符（无前导空白跳过）。
                // %lc（l 修饰）目标为 wchar_t*（x86_64 上 4 字节 i32），每字节转宽字符。
                if conv == b'c' {
                    let wide = len == FScanLen::L;
                    let target8: *mut u8 = if suppress {
                        core::ptr::null_mut()
                    } else {
                        ap.next_arg::<usize>() as *mut u8
                    };
                    let target32 = target8 as *mut i32;
                    // %c/%lc 无显式宽度时默认读 1 个字符（C 语义）。
                    let cw = if width == 0 { 1 } else { w };
                    let mut k = 0usize;
                    while k < cw {
                        let ch = fscan_getc(f);
                        if ch < 0 {
                            break;
                        }
                        consumed += 1;
                        if !target8.is_null() {
                            if wide {
                                *target32.add(k) = ch as i32;
                            } else {
                                *target8.add(k) = ch as u8;
                            }
                        }
                        k += 1;
                    }
                    if k == 0 {
                        break;
                    }
                    if !suppress {
                        assigned += 1;
                    }
                    continue;
                }

                // %[ 扫描集：读直到集合外字符。格式：%[^]...] 或 %[]...]。
                if conv == b'[' {
                    // 解析扫描集（在 fmt 里找闭合的 ']'）。
                    let mut negate = false;
                    let mut set_lo = i;
                    if i < fmt_bytes.len() && fmt_bytes[i] == b'^' {
                        negate = true;
                        set_lo += 1;
                    }
                    // 找闭合 ']'。
                    let mut set_hi = set_lo;
                    let mut closed = false;
                    while set_hi < fmt_bytes.len() {
                        if fmt_bytes[set_hi] == b']' {
                            // ']' 若在首位则视为集合成员，继续找。
                            if set_hi == set_lo {
                                set_hi += 1;
                                continue;
                            }
                            closed = true;
                            break;
                        }
                        set_hi += 1;
                    }
                    if !closed {
                        break;
                    }
                    let set_members = &fmt_bytes[set_lo..set_hi];
                    i = set_hi + 1;
                    let target: *mut u8 = if suppress {
                        core::ptr::null_mut()
                    } else {
                        ap.next_arg::<usize>() as *mut u8
                    };
                    let mut k = 0usize;
                    let mut cur = fscan_first_non_white(f);
                    loop {
                        if cur < 0 {
                            break;
                        }
                        if k >= w {
                            break;
                        }
                        let in_set = set_members.contains(&(cur as u8));
                        let keep = if negate { !in_set } else { in_set };
                        if !keep {
                            fscan_ungetc(f, cur); // 回退不匹配字符。
                            break;
                        }
                        consumed += 1;
                        if !target.is_null() {
                            *target.add(k) = cur as u8;
                        }
                        k += 1;
                        cur = fscan_getc(f);
                    }
                    if k == 0 {
                        break;
                    }
                    if !target.is_null() {
                        *target.add(k) = 0;
                        assigned += 1;
                    }
                    continue;
                }

                // 其余转换：跳过前导空白，读入首个非空白字符 first。
                let mut first = fscan_getc(f);
                if first < 0 {
                    break; // EOF 或错误。
                }
                while fscan_isspace(first) {
                    first = fscan_getc(f);
                    if first < 0 {
                        break;
                    }
                }
                if first < 0 {
                    break; // token 前即 EOF。
                }
                consumed += 1; // first 已消费。

                // 以 first 作为 token 首字符，按转换类型解析。
                match conv {
                    b'd' | b'i' => {
                        // 符号。
                        let mut neg = false;
                        let mut first_val = first;
                        if first == b'-' as i32 {
                            neg = true;
                            first_val = fscan_getc(f);
                            if first_val < 0 { break; }
                            consumed += 1;
                        } else if first == b'+' as i32 {
                            first_val = fscan_getc(f);
                            if first_val < 0 { break; }
                            consumed += 1;
                        }
                        // 数字累积。
                        let mut acc: u64 = 0;
                        let mut any = false;
                        let mut nread = 0usize;
                        let mut cur = first_val;
                        loop {
                            if nread >= w {
                                break;
                            }
                            let d = if cur < 0 { None } else { (cur as u8 as char).to_digit(10) };
                            match d {
                                Some(dv) => {
                                    acc = acc.wrapping_mul(10).wrapping_add(dv as u64);
                                    any = true;
                                    nread += 1;
                                }
                                None => {
                                    if cur >= 0 { fscan_ungetc(f, cur); } // 回退非数字。
                                    break;
                                }
                            }
                            if nread >= w {
                                break;
                            }
                            cur = fscan_getc(f);
                            if cur < 0 { break; }
                            consumed += 1;
                        }
                        if !any {
                            break;
                        }
                        let value = if neg { acc.wrapping_neg() as i64 } else { acc as i64 };
                        if !suppress {
                            let p = ap.next_arg::<usize>() as *mut u8;
                            if !p.is_null() {
                                write_int_width(p, value, &len);
                                assigned += 1;
                            }
                        }
                    }
                    b'u' | b'x' | b'o' => {
                        let base: u32 = match conv {
                            b'u' => 10,
                            b'x' => 16,
                            _ => 8,
                        };
                        let mut acc: u64 = 0;
                        let mut any = false;
                        let mut nread = 0usize;
                        let mut cur = first;
                        loop {
                            if nread >= w {
                                break;
                            }
                            let d = if cur < 0 { None } else { (cur as u8 as char).to_digit(base) };
                            match d {
                                Some(dv) => {
                                    acc = acc.wrapping_mul(base as u64).wrapping_add(dv as u64);
                                    any = true;
                                    nread += 1;
                                }
                                None => {
                                    if cur >= 0 { fscan_ungetc(f, cur); } // 回退非本基数数字。
                                    break;
                                }
                            }
                            if nread >= w {
                                break;
                            }
                            cur = fscan_getc(f);
                            if cur < 0 { break; }
                            consumed += 1;
                        }
                        if !any {
                            break;
                        }
                        if !suppress {
                            let p = ap.next_arg::<usize>() as *mut u8;
                            if !p.is_null() {
                                write_uint_width(p, acc, &len);
                                assigned += 1;
                            }
                        }
                    }
                    b's' => {
                        // %ls（l 修饰）目标为 wchar_t*，每字节转宽字符（i32）。
                        let wide = len == FScanLen::L;
                        let target: *mut u8 = if suppress {
                            core::ptr::null_mut()
                        } else {
                            ap.next_arg::<usize>() as *mut u8
                        };
                        let target32 = target as *mut i32;
                        let mut k = 0usize;
                        let mut cur = first;
                        loop {
                            if cur < 0 {
                                break;
                            }
                            if fscan_isspace(cur) {
                                fscan_ungetc(f, cur); // 回退结束 %s 的空白字符。
                                break;
                            }
                            if k >= w {
                                fscan_ungetc(f, cur); // 宽度用尽，回退超宽字符。
                                break;
                            }
                            if !target.is_null() {
                                if wide {
                                    *target32.add(k) = cur as i32;
                                } else {
                                    *target.add(k) = cur as u8;
                                }
                            }
                            k += 1;
                            cur = fscan_getc(f);
                            if cur >= 0 {
                                consumed += 1;
                            }
                        }
                        if k == 0 {
                            break;
                        }
                        if !target.is_null() {
                            if wide {
                                *target32.add(k) = 0;
                            } else {
                                *target.add(k) = 0;
                            }
                            assigned += 1;
                        }
                    }
                    b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => {
                        // 实数 tokenizer：<symbol> dig.dig [e[sign]dig]。
                        let mut neg = false;
                        let mut cur = first;
                        if cur == b'-' as i32 {
                            neg = true;
                            cur = fscan_getc(f);
                            if cur < 0 { break; }
                            consumed += 1;
                        } else if cur == b'+' as i32 {
                            cur = fscan_getc(f);
                            if cur < 0 { break; }
                            consumed += 1;
                        }
                        let mut mant: u64 = 0;
                        let mut mant_digits: u32 = 0;
                        let mut frac_digits: u32 = 0;
                        let mut any = false;
                        let mut seen_dot = false;
                        let mut nread = 0usize;
                        loop {
                            if nread >= w {
                                break;
                            }
                            if cur == b'.' as i32 && !seen_dot {
                                seen_dot = true;
                                nread += 1;
                                cur = fscan_getc(f);
                                if cur >= 0 { consumed += 1; }
                                continue;
                            }
                            let d = if cur < 0 { None } else { (cur as u8 as char).to_digit(10) };
                            match d {
                                Some(dv) => {
                                    any = true;
                                    nread += 1;
                                    if seen_dot {
                                        frac_digits += 1;
                                    }
                                    if mant_digits < 19 {
                                        mant = mant.wrapping_mul(10).wrapping_add(dv as u64);
                                        mant_digits += 1;
                                    }
                                }
                                None => break,
                            }
                            cur = fscan_getc(f);
                            if cur >= 0 { consumed += 1; }
                        }
                        if !any {
                            break;
                        }
                        let mut exp10: i32 = 0;
                        if cur == b'e' as i32 || cur == b'E' as i32 {
                            cur = fscan_getc(f);
                            if cur >= 0 { consumed += 1; }
                            let mut eneg = false;
                            if cur == b'-' as i32 {
                                eneg = true;
                                cur = fscan_getc(f);
                                if cur >= 0 { consumed += 1; }
                            } else if cur == b'+' as i32 {
                                cur = fscan_getc(f);
                                if cur >= 0 { consumed += 1; }
                            }
                            let mut ed: i32 = 0;
                            let mut eany = false;
                            while (0..10).contains(&ed) {
                                let d = if cur < 0 { None } else { (cur as u8 as char).to_digit(10) };
                                match d {
                                    Some(dv) => {
                                        eany = true;
                                        ed = ed.saturating_mul(10).saturating_add(dv as i32);
                                        cur = fscan_getc(f);
                                        if cur >= 0 { consumed += 1; }
                                    }
                                    None => break,
                                }
                            }
                            if eany {
                                exp10 = if eneg { -ed } else { ed };
                            }
                            // 指数部分结束，回退非数字终止字符。
                            if cur >= 0 {
                                fscan_ungetc(f, cur);
                            }
                        } else {
                            // 无指数：回退终止 mantissa 的非数字字符。
                            if cur >= 0 {
                                fscan_ungetc(f, cur);
                            }
                        }
                        let mut value: f64 = mant as f64;
                        let adjust = exp10 as i64 - frac_digits as i64;
                        if adjust > 308 {
                            value = f64::INFINITY;
                        } else if adjust < -324 {
                            value = 0.0;
                        } else {
                            // value = mant × 10^adjust。adjust 已含小数点位移（exp10 - frac_digits），
                            // 故**两个方向都是同一个公式**——此前负数分支多取了一次负号，
                            // 把 "3.5"（mant=35, frac_digits=1, adjust=-1）算成 35×10¹ = 350。
                            // 这是 fscanf 里的**既有**缺陷，sscanf 复用它才暴露出来。
                            value *= f64_pow10(adjust as i32);
                        }
                        if neg {
                            value = -value;
                        }
                        if !suppress {
                            // 目标指针宽度：%f→float*、%lf→double*、%Lf→long double*。
                            let p = ap.next_arg::<usize>() as *mut u8;
                            if !p.is_null() {
                                match &len {
                                    FScanLen::L => {
                                        let q = p as *mut f64;
                                        *q = value;
                                    }
                                    _ => {
                                        let q = p as *mut f32;
                                        *q = value as f32;
                                    }
                                }
                                assigned += 1;
                            }
                        }
                    }
                    _ => { break; }
                }
            } else if fscan_isspace(c as i32) {
                // 格式串空白：匹配任意输入空白（含 0 个）。
                // 读完空白后须回退首个非空白字符，否则该字符被吞掉导致后续转换错位。
                let mut sc = fscan_getc(f);
                while sc >= 0 && fscan_isspace(sc) {
                    sc = fscan_getc(f);
                }
                if sc >= 0 {
                    fscan_ungetc(f, sc);
                }
                i += 1;
            } else {
                // 字面量：读一个字符须相等。
                let ch = fscan_getc(f);
                if ch != c as i32 {
                    break;
                }
                consumed += 1;
                i += 1;
            }
        }
    assigned
}

/// `scanf(fmt, ...)`：从**标准输入**读取格式化输入（POSIX）。
///
/// 与 `fscanf` 共用同一套扫描核心 `vscan`（S15 单点）——本函数只是把流换成 `stdin`。
/// **来路**：反向对账的 C1 清单（`sscanf`/`fscanf` 早已实现，缺的只是「从 stdin 读」这一层）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scanf(fmt: *const c_char, ap: ...) -> c_int {
    use core::ffi::VaList;
    stdio_init();
    if fmt.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    let fp = stdin;
    if fp.is_null() {
        set_errno(EBADF);
        return EOF;
    }
    let ap: VaList = core::mem::transmute(ap);
    vscan(&mut *fp, fmt, ap)
}

/// `tmpfile()`：创建一个**自动删除**的临时文件并返回其流（POSIX，`"w+b"` 语义）。
///
/// 实现是真实数据链路，不是近似：
/// 1. 目录取 `TMPDIR`（非空时）否则 `/tmp`——本系统里 `/tmp` 是指向 `/scratch` 的符号链接
///    （内核 `vfs_init`），故它始终存在且可写；
/// 2. 用 `mkstemp` 以 **O_EXCL 原子独占**建名并打开（撞名换名重试；判定与创建同在内核
///    一个 syscall 内，无 TOCTOU 窗口）；
/// 3. **立即 `unlink`**——POSIX 要求文件在关闭或进程结束时自动消失。本内核的「打开后删除」
///    是延迟生命周期（`[test-vfs-m65] open-unlink deferred lifecycle OK` 已验证），故 fd 仍可用；
/// 4. `alloc_file(fd, FmMode::ReadWrite)` 包成**可读可写**流（这正是 `FmMode` 新增
///    `ReadWrite` 变体的唯一动机：此前没有它，`fdopen(fd, "w+b")` 只会得到**只写**流）。
///
/// **诚实边界（S09）**：目录不存在/不可写、或 fd 表满时如实返回 NULL 并置 errno——
/// **不伪造**成功。POSIX 未规定 tmpfile 失败的具体 errno，故原样透传内核错误码。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tmpfile() -> *mut FILE {
    // 1. 目录：TMPDIR 优先（POSIX 惯例），否则 /tmp。
    let mut dir_buf = [0u8; 96];
    let mut dlen = 0usize;
    let env = crate::stdlib::getenv(b"TMPDIR\0".as_ptr() as *const c_char);
    if !env.is_null() {
        let b = cstr_bytes(env);
        if !b.is_empty() && b.len() + 11 < dir_buf.len() {
            dir_buf[..b.len()].copy_from_slice(b);
            dlen = b.len();
        }
    }
    if dlen == 0 {
        let d = b"/tmp";
        dir_buf[..d.len()].copy_from_slice(d);
        dlen = d.len();
    }
    // 2. 拼 "<dir>/tmpXXXXXX"（结尾必须是 6 个 'X'——mkstemp 的模板契约）。
    let mut tpl = [0i8; 128];
    let mut n = 0usize;
    let mut dl = dlen;
    while dl > 1 && dir_buf[dl - 1] == b'/' {
        dl -= 1;
    }
    for &b in dir_buf[..dl].iter() {
        tpl[n] = b as i8;
        n += 1;
    }
    for &b in b"/tmpXXXXXX".iter() {
        tpl[n] = b as i8;
        n += 1;
    }
    tpl[n] = 0;
    let fd = crate::posix_batch3::mkstemp(tpl.as_mut_ptr());
    if fd < 0 {
        return core::ptr::null_mut();
    }
    // 3. 登记**延迟删除**（理由见 TMP_FILES 的说明：本内核 VFS 不支持 unlink 之后再写入）。
    let path_bytes = core::slice::from_raw_parts(tpl.as_ptr() as *const u8, n);
    if core::str::from_utf8(path_bytes).is_err() {
        let _ = crate::unistd::close(fd);
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let copy = crate::malloc::malloc(n + 1) as *mut c_char;
    if copy.is_null() {
        let _ = crate::unistd::close(fd);
        set_errno(crate::errno::ENOMEM);
        return core::ptr::null_mut();
    }
    core::ptr::copy_nonoverlapping(path_bytes.as_ptr(), copy as *mut u8, n);
    *copy.add(n) = 0;
    if !tmp_register(fd, copy) {
        // 登记表满：如实失败，**不留半成品**（删掉刚建的文件 + 释放路径 + 关 fd）。
        if let Ok(p) = core::str::from_utf8(path_bytes) {
            let _ = libsys::unlink(p);
        }
        crate::malloc::free(copy as *mut u8);
        let _ = crate::unistd::close(fd);
        set_errno(crate::errno::ENOMEM);
        return core::ptr::null_mut();
    }
    if !TMP_CLEANUP_REGISTERED.swap(true, Ordering::SeqCst) {
        let _ = crate::stdlib::atexit(Some(tmp_cleanup_at_exit));
    }
    // 4. 包成读写流。
    let fp = alloc_file(fd as u64, FmMode::ReadWrite);
    if fp.is_null() {
        let _ = crate::unistd::close(fd);
        return core::ptr::null_mut();
    }
    fp
}

