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
use core::sync::atomic::{AtomicBool, Ordering};

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
pub enum FmMode { Read, Write, Append }

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

/// 初始化标准流（幂等，可多次调用）。
pub fn stdio_init() {
    lock();
    unsafe {
        static STREAM_STDIN: SyncStream = SyncStream::new(FILE {
            fd: 0, mode: FmMode::Read, buf_mode: FmBufMode::None, eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false, pushback: -1,
        });
        static STREAM_STDOUT: SyncStream = SyncStream::new(FILE {
            fd: 1, mode: FmMode::Write, buf_mode: FmBufMode::None, eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false, pushback: -1,
        });
        static STREAM_STDERR: SyncStream = SyncStream::new(FILE {
            fd: 2, mode: FmMode::Write, buf_mode: FmBufMode::None, eof: false, error: false,
            buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false, pushback: -1,
        });
        if stdin.is_null() {
            stdin = STREAM_STDIN.get();
            stdout = STREAM_STDOUT.get();
            stderr = STREAM_STDERR.get();
        }
    }
    unlock();
}

fn parse_mode(mode: *const c_char) -> Option<FmMode> {
    unsafe {
        let c0 = *mode as u8;
        match c0 {
            b'r' => Some(FmMode::Read),
            b'w' => Some(FmMode::Write),
            b'a' => Some(FmMode::Append),
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
    let fm = match parse_mode(mode) {
        Some(m) => m,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    let path_str = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => { set_errno(EINVAL); return core::ptr::null_mut(); }
    };
    let flags = match fm {
        FmMode::Read => libsys::OpenFlags::READ_ONLY,
        FmMode::Write => libsys::OpenFlags::CREATE_OR_TRUNCATE,
        FmMode::Append => libsys::OpenFlags {
            read: false, write: true, create: true, truncate: false, append: true, directory: false, pipe: false,
        },
    };
    let perm = libsys::Permissions::read_write();
    match libsys::open(path_str, flags, perm) {
        Ok(fd) => {
            let fp = crate::malloc::malloc(core::mem::size_of::<FILE>()) as *mut FILE;
            if fp.is_null() {
                let _ = libsys::close(fd);
                set_errno(crate::errno::ENOMEM);
                return core::ptr::null_mut();
            }
            unsafe {
                core::ptr::write(fp, FILE {
                    fd, mode: fm, buf_mode: FmBufMode::None, eof: false, error: false,
                    buf_ptr: core::ptr::null_mut(), buf_len: 0, buf_pos: 0, closed: false,
                    pushback: -1,
                });
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
        let fd = f.fd;
        f.closed = true;
        let _ = libsys::close(fd);
        crate::malloc::free(fp as *mut u8);
    }
    0
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
        if f.mode != FmMode::Read {
            set_errno(EINVAL);
            f.error = true;
            return 0;
        }
        let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, total);
        match libsys::read(f.fd, buf) {
            Ok(n) => {
                if n < total {
                    f.eof = true;
                }
                n / size
            }
            Err(e) => {
                set_errno(from_libsys(e));
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
        if f.mode == FmMode::Read {
            set_errno(EINVAL);
            f.error = true;
            return 0;
        }
        let buf = core::slice::from_raw_parts(ptr as *const u8, total);
        match libsys::write(f.fd, buf) {
            Ok(n) => n / size,
            Err(e) => {
                set_errno(from_libsys(e));
                f.error = true;
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fflush(_fp: *mut FILE) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetc(fp: *mut FILE) -> c_int {
    unsafe {
        let f = &mut *fp;
        if f.mode != FmMode::Read {
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
        match libsys::read(f.fd, &mut b) {
            Ok(0) => { f.eof = true; EOF }
            Ok(_) => b[0] as c_int,
            Err(e) => {
                set_errno(from_libsys(e));
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
        if f.mode != FmMode::Read {
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
        if f.mode == FmMode::Read {
            f.error = true;
            set_errno(EINVAL);
            return EOF;
        }
        let b = [(c & 0xFF) as u8; 1];
        match libsys::write(f.fd, &b) {
            Ok(_) => c & 0xFF,
            Err(e) => {
                set_errno(from_libsys(e));
                f.error = true;
                EOF
            }
        }
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
        if f.mode != FmMode::Read {
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
            match libsys::read(f.fd, &mut b) {
                Ok(0) => {
                    f.eof = true;
                    break;
                }
                Ok(_) => {
                    *s.add(i) = b[0] as c_char;
                    i += 1;
                    if b[0] == b'\n' {
                        break;
                    }
                }
                Err(e) => {
                    set_errno(from_libsys(e));
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
        if f.mode != FmMode::Read {
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
        if f.mode == FmMode::Read {
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
        match libsys::write(f.fd, buf) {
            Ok(_) => 0,
            Err(e) => {
                set_errno(from_libsys(e));
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fseek(_fp: *mut FILE, _offset: c_long, _whence: c_int) -> c_int {
    // 定位未完整支持（内核 fd 语义下顺序流为主）：如实 ENOTSUP，不伪造成功。
    set_errno(crate::errno::ENOTSUP);
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftell(_fp: *mut FILE) -> c_long {
    set_errno(crate::errno::ENOTSUP);
    -1
}

// ---------- 内部辅助 ----------

/// 把 C 字符串复制为 Rust 字节 Vec（直到 NUL）。
pub unsafe fn cstr_to_bytes(p: *const c_char) -> alloc::vec::Vec<u8> {
    unsafe {
        let mut v = alloc::vec::Vec::new();
        let mut q = p;
        while *q != 0 {
            v.push(*q as u8);
            q = q.add(1);
        }
        v
    }
}

/// 把 C 字符串转换为 Rust `&str`（用于 libsys 的 &str 参数）。
/// 非法 UTF-8 返回 None（调用方置 EINVAL，S02 显式处理编码）。
pub unsafe fn cstr_to_str(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    let bytes = unsafe { cstr_to_bytes(p) };
    // 泄漏为 'static（进程生命周期内恒定）。
    core::str::from_utf8(alloc::boxed::Box::leak(bytes.into_boxed_slice())).ok()
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
}
// ---------- printf 核心（经 VaList 读取可变参数） ----------

use core::ffi::VaList;

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
                let bytes = unsafe { cstr_to_bytes(p) };
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
            let v = unsafe { ap.next_arg::<f64>() };
            let precision = if s.prec >= 0 { s.prec as usize } else { 6 };
            let mut d = crate::float::decompose(v);
            match s.conv {
                Conv::Float => crate::float::emit_fixed(&s, &mut d, precision, sink),
                Conv::Exp => crate::float::emit_exp(&s, &mut d, precision, sink),
                _ => crate::float::emit_general(&s, &mut d, precision, sink),
            }
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
    let fmt_bytes = unsafe { cstr_to_bytes(fmt) };
    let mut emitted: usize = 0;
    parse_and_format(&fmt_bytes, sink, |spec, sink| {
        let before = emitted;
        render_spec(spec, before, ap, sink)?;
        emitted += 1;
        Ok(())
    })?;
    Ok(emitted as ssize_t)
}

/// 核心：格式化到内存缓冲（sprintf/snprintf），返回应写字节数。
fn vformat_mem(
    fmt: *const c_char,
    ap: &mut VaList,
    buf: *mut u8,
    cap: usize,
) -> Result<ssize_t, ()> {
    if cap == 0 || buf.is_null() {
        // 只统计长度（snprintf cap=0 合法）。
        let fmt_bytes = unsafe { cstr_to_bytes(fmt) };
        let mut counter = CounterSink { n: 0 };
        let mut emitted = 0usize;
        parse_and_format(&fmt_bytes, &mut counter, |spec, sink| {
            let before = emitted;
            render_spec(spec, before, ap, sink)?;
            emitted += 1;
            Ok(())
        })?;
        return Ok(counter.n as ssize_t);
    }
    let mut mem = MemSink {
        buf: unsafe { core::slice::from_raw_parts_mut(buf, cap) },
        pos: 0,
        truncated: false,
    };
    let fmt_bytes = unsafe { cstr_to_bytes(fmt) };
    let mut emitted = 0usize;
    parse_and_format(&fmt_bytes, &mut mem, |spec, sink| {
        let before = emitted;
        render_spec(spec, before, ap, sink)?;
        emitted += 1;
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
}
impl FdSink {
    fn flush(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match libsys::write(self.fd, bytes) {
            Ok(n) => self.wrote += n as ssize_t,
            Err(e) => { set_errno(from_libsys(e)); }
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
    if f.mode == FmMode::Read {
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
        let fmt_bytes = unsafe { cstr_to_bytes(fmt) };
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
    let bytes = unsafe { cstr_to_bytes(s) };
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
    match libsys::write(1, &b) {
        Ok(_) => c & 0xFF,
        Err(e) => {
            set_errno(from_libsys(e));
            EOF
        }
    }
}

/// \`getchar()\`：从 stdin 读一字符；EOF 或错误返回 EOF。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getchar() -> c_int {
    stdio_init();
    let mut b = [0u8; 1];
    match libsys::read(0, &mut b) {
        Ok(0) => EOF,
        Ok(_) => b[0] as c_int,
        Err(e) => {
            set_errno(from_libsys(e));
            EOF
        }
    }
}
// ---------- 格式化输入（fscanf 最小子集） ----------

/// 内部：从 FILE 读一个字节；EOF 返回 -1。
unsafe fn fscan_getc(f: &mut FILE) -> i32 {
    // 优先读回 pushback 槽。
    if f.pushback >= 0 {
        let c = f.pushback;
        f.pushback = -1;
        return c;
    }
    let mut b = [0u8; 1];
    match libsys::read(f.fd, &mut b) {
        Ok(0) => { f.eof = true; -1 }
        Ok(_) => b[0] as i32,
        Err(e) => { set_errno(from_libsys(e)); f.error = true; -1 }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fscanf(fp: *mut FILE, fmt: *const c_char, ap: ...) -> c_int {
    use core::ffi::VaList;
    if fp.is_null() || fmt.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    unsafe {
        let f = &mut *fp;
        if f.mode != FmMode::Read {
            f.error = true;
            set_errno(EINVAL);
            return EOF;
        }
        let mut ap: VaList = core::mem::transmute(ap);
        let fmt_bytes = crate::stdio::cstr_to_bytes(fmt);
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
                    let mut k = 0usize;
                    while k < w {
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
                            if adjust < 0 {
                                value *= f64_pow10(-adjust as i32);
                            } else {
                                value *= f64_pow10(adjust as i32);
                            }
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
                while fscan_isspace(fscan_getc(f)) {}
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
}


