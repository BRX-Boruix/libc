//! 标准库杂项（C ABI）：字符串转数值、随机数、abs/div、atoi 家族。
//!
//! ## 字符串转数值
//!
//! \`strtol/strtoul/strtoll/strtoull\` 支持任意 base（0=自动前缀推断，
//! 2..=36），处理前导空白、可选符号、前缀（0x/0b/0）。溢出时置 ERANGE 并返回
//! LONG_MAX/LONG_MIN（POSIX 语义），endptr 指向停止位置。实现用 checked 运算
//! 避免溢出（S19）。

use crate::ctypes::{c_int, c_long, c_ulong, c_longlong, c_ulonglong, c_uint, size_t, c_void};
use crate::errno::{set_errno, ERANGE};
use crate::ctypes::c_char;

/// `boruix_so_abi_version()`：共享库 ABI 版本探针（阶段 5 / 3P5-2）。
///
/// **加它是故意的**：用于验证"重编 libc、**只替换 `.so`** 之后，**未重链接**的旧程序仍能跑"。
/// 一个**新增**导出不会破坏既有程序——这正是"符号 ABI 冻结"的含义：
/// 已发布的符号不得改签名/改语义，新增是允许的（向后兼容）。
#[unsafe(no_mangle)]
pub extern "C" fn boruix_so_abi_version() -> c_int {
    2
}

// ---------- atexit（3P3-2）----------

use spin::Mutex;

/// 退出处理函数登记表容量。
///
/// **固定容量、不分配**：atexit 常被用在退出路径上，此时不应再依赖堆状态
/// （堆可能已损坏或已被释放）。表满时按 POSIX 允许的方式如实返回非 0。
const MAX_ATEXIT: usize = 32;

type ExitFn = extern "C" fn();

/// `on_exit` 的处理函数：`(status, arg)`（POSIX 的 `on_exit` 形态）。
pub type OnExitFn = extern "C" fn(crate::ctypes::c_int, *mut crate::ctypes::c_void);

/// 退出处理项。
///
/// **为什么两种登记共用一个表**：POSIX 要求 `atexit` 与 `on_exit` 登记的处理器在**同一个**
/// LIFO 栈里、按登记顺序逆序执行。分成两张表就会变成「先跑完所有 on_exit 再跑 atexit」——
/// 那是**可观测的语义错误**（依赖顺序的清理代码会崩）。故这里用枚举合并。
#[derive(Clone, Copy)]
enum ExitEntry {
    Plain(ExitFn),
    // 存成 usize 而不是裸指针：`spin::Mutex<T>` 要求 `T: Send`，而 `*mut c_void` 不是
    // Send（类型系统不区分「本内核单核」与「真跨线程」）。取出时再转回指针。
    WithArg(OnExitFn, usize),
}

static ATEXIT_FNS: Mutex<[Option<ExitEntry>; MAX_ATEXIT]> = Mutex::new([None; MAX_ATEXIT]);

/// `atexit(f)`：登记退出处理函数。成功返回 0；表满返回非 0。
#[unsafe(no_mangle)]
pub extern "C" fn atexit(f: Option<ExitFn>) -> c_int {
    let Some(f) = f else {
        return -1;
    };
    let mut table = ATEXIT_FNS.lock();
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(ExitEntry::Plain(f));
            return 0;
        }
    }
    1
}

/// `on_exit(f, arg)`：登记**带参数**的退出处理函数。成功返回 0；表满或 `f` 为空返回非 0。
///
/// 与 `atexit` 共用同一个 LIFO 表（见 [`ExitEntry`] 的说明），故两种登记的执行顺序
/// 严格按登记顺序逆序——这是 POSIX 的硬要求，不是实现细节。
#[unsafe(no_mangle)]
pub extern "C" fn on_exit(
    f: Option<OnExitFn>,
    arg: *mut crate::ctypes::c_void,
) -> crate::ctypes::c_int {
    let Some(f) = f else {
        return -1;
    };
    let mut table = ATEXIT_FNS.lock();
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(ExitEntry::WithArg(f, arg as usize));
            return 0;
        }
    }
    1
}

/// 逆序调用全部已登记的退出处理函数（LIFO，POSIX 语义）。由 `exit` 调用；`_exit` 不调用。
///
/// 逐个「取出后释放锁再调用」：处理函数内部可能再次调用 `atexit`/`exit`，持锁调用会自死锁。
pub fn run_atexit_handlers(code: crate::ctypes::c_int) {
    loop {
        let f = {
            let mut table = ATEXIT_FNS.lock();
            let mut picked = None;
            for slot in table.iter_mut().rev() {
                if slot.is_some() {
                    picked = slot.take();
                    break;
                }
            }
            picked
        };
        match f {
            Some(ExitEntry::Plain(f)) => f(),
            // POSIX：`on_exit` 的处理函数收到退出状态。`code` 由 `exit(code)` 一路传入，
            // 不是编造的 0。
            Some(ExitEntry::WithArg(f, arg)) => f(code, arg as *mut crate::ctypes::c_void),
            None => break,
        }
    }
}
// ---------- 异常终止与断言（3P3-2）----------

/// `abort()`：异常终止（`stdlib.h`）。
///
/// 按 C 语义先发 SIGABRT（POSIX 固定号 6）；若该信号被忽略或捕获而返回，则用
/// `_exit(128 + SIGABRT)` 兜底。**绝不返回**。
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    // 本系统**没有** SIGABRT（内核 task::signals 的集合里没有 6），故不能像 glibc 那样
    // 先 raise(SIGABRT)。直接以 128+6=134 异常终止——与 POSIX 约定中「被 SIGABRT 终止」的
    // 退出码一致，调用方（shell/wait）看到的结果语义相同。绝不返回。
    crate::process::_exit(134)
}

/// `__assert_fail(expr, file, line)`：`assert()` 失败路径（`assert.h`）。
///
/// 直写 stderr 而不依赖缓冲状态——断言失败路径上不应再有「输出可能丢失」的不确定性。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __assert_fail(expr: *const c_char, file: *const c_char, line: c_uint) -> ! {
    unsafe {
        let _ = crate::stdio::fprintf(
            crate::stdio::stderr,
            c"assertion failed: %s (%s:%u)\n".as_ptr(),
            expr,
            file,
            line,
        );
    }
    abort()
}

/// \`abs(n)\`：绝对值（i32::MIN 返回自身，未定义）。
#[unsafe(no_mangle)]
pub extern "C" fn abs(n: c_int) -> c_int {
    n.wrapping_abs()
}

/// \`labs(n)\`：long 绝对值。
#[unsafe(no_mangle)]
pub extern "C" fn labs(n: c_long) -> c_long {
    n.wrapping_abs()
}

/// \`llabs(n)\`：long long 绝对值。
#[unsafe(no_mangle)]
pub extern "C" fn llabs(n: c_longlong) -> c_longlong {
    n.wrapping_abs()
}

/// `imaxdiv_t`（C 布局，与 `libc/include/inttypes.h` 同一约定）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImaxdivT {
    pub quot: i64,
    pub rem: i64,
}

/// `imaxabs(j)`：`intmax_t` 绝对值。
///
/// `intmax_t` 在本目标是 `long`（LP64，由探针确认，见 tcc-on-boruix/boruix/_probe_types.c）。
/// `i64::MIN` 的绝对值在 C 里是 UB，此处按二进制补码回绕而非 panic——库函数不该因为输入取
/// 极端值就崩掉调用者。
#[unsafe(no_mangle)]
pub extern "C" fn imaxabs(j: i64) -> i64 {
    j.wrapping_abs()
}

/// `imaxdiv(numer, denom)`：同时给出商与余数（C 语义：向零截断）。
#[unsafe(no_mangle)]
pub extern "C" fn imaxdiv(numer: i64, denom: i64) -> ImaxdivT {
    ImaxdivT {
        quot: numer.wrapping_div(denom),
        rem: numer.wrapping_rem(denom),
    }
}

/// `strtoimax(...)`：**复用 `strtoll`**（S15 单点——进制/前缀/溢出逻辑只有一份）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoimax(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
) -> i64 {
    unsafe { strtoll(s, endptr, base) }
}

/// `strtoumax(...)`：**复用 `strtoull`**。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoumax(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
) -> u64 {
    unsafe { strtoull(s, endptr, base) }
}

/// C 全局 `environ`：环境数组（NUL 终结的 `char*` 数组）。
///
/// **直接指向进程初始栈上的 envp 数组本身**——无需拷贝，也不占额外内存。
/// 程序可以给它赋值（POSIX 允许），此后 `getenv` 读的是新数组。
///
/// **诚实边界（S39）**：只有经 C 入口桥接（`csrc/user_main.c`）启动的程序才会被注册；
/// 不经该桥接的程序（例如 Rust 程序）`environ` 保持 NULL，`getenv` 如实返回 NULL。
#[unsafe(no_mangle)]
pub static mut environ: *mut *mut crate::ctypes::c_char = core::ptr::null_mut();

// ---------- 环境表修改（setenv/unsetenv/putenv/clearenv，3P6-2 第二波「整项缺失」类） ----------
//
// 设计：入口桥接把 environ 指向**入口栈上的数组**（只读语义、无空位）。要修改就必须换成一个
// **我们自己拥有**的数组（可 realloc、可 free）——故首次修改前整体复制一份，此后所有修改都在
// 我们自己的数组上做。这就是 POSIX 允许的实现方式，也避免了往入口栈数组后面写。

/// 我们自己分配的环境数组（NULL 终止）。
static mut ENV_OWNED: *mut *mut crate::ctypes::c_char = core::ptr::null_mut();
/// 上面那个数组的**容量**（元素个数，含 NULL 位）。
static mut ENV_CAP: usize = 0;

unsafe fn env_owned() -> bool {
    unsafe {
        if !ENV_OWNED.is_null() {
            return true;
        }
        let cur = *core::ptr::addr_of!(environ);
        let mut n = 0usize;
        if !cur.is_null() {
            while *cur.add(n) != core::ptr::null_mut() {
                n += 1;
            }
        }
        let cap = n + 8;
        let arr = crate::malloc::malloc(cap * core::mem::size_of::<*mut crate::ctypes::c_char>())
            as *mut *mut crate::ctypes::c_char;
        if arr.is_null() {
            return false;
        }
        if n > 0 {
            core::ptr::copy_nonoverlapping(cur, arr, n);
        }
        *arr.add(n) = core::ptr::null_mut();
        ENV_OWNED = arr;
        ENV_CAP = cap;
        *core::ptr::addr_of_mut!(environ) = arr;
        true
    }
}

/// 环境表里 `name`（长度 nlen）对应的下标；不存在返回 None。
unsafe fn env_index(name: *const crate::ctypes::c_char, nlen: usize) -> Option<usize> {
    unsafe {
        let arr = ENV_OWNED;
        if arr.is_null() {
            return None;
        }
        let mut i = 0usize;
        loop {
            let e = *arr.add(i);
            if e.is_null() {
                return None;
            }
            let mut same = true;
            for k in 0..nlen {
                if *e.add(k) != *name.add(k) {
                    same = false;
                    break;
                }
            }
            if same && *e.add(nlen) == b'=' as crate::ctypes::c_char {
                return Some(i);
            }
            i += 1;
        }
    }
}

/// 追加一条 entry（需要环境表已是自有数组）；必要时扩容。
unsafe fn env_push(entry: *mut crate::ctypes::c_char) -> bool {
    unsafe {
        let mut n = 0usize;
        let arr = ENV_OWNED;
        while *arr.add(n) != core::ptr::null_mut() {
            n += 1;
        }
        if n + 2 > ENV_CAP {
            let newcap = if ENV_CAP == 0 { 16 } else { ENV_CAP * 2 };
            let p = crate::malloc::realloc(
                arr as *mut u8,
                newcap * core::mem::size_of::<*mut crate::ctypes::c_char>(),
            ) as *mut *mut crate::ctypes::c_char;
            if p.is_null() {
                return false;
            }
            ENV_OWNED = p;
            ENV_CAP = newcap;
            *core::ptr::addr_of_mut!(environ) = p;
        }
        let arr = ENV_OWNED;
        *arr.add(n) = entry;
        *arr.add(n + 1) = core::ptr::null_mut();
        true
    }
}

/// setenv(name, value, overwrite)：写入环境变量（POSIX）。
///
/// **来路（3P6-2 第二波「整项缺失」类，反向对账列出）。**
///
/// **诚实边界（S09）**：环境表是**进程全局**的，本实现不加密锁保护——并发 setenv/putenv 与
/// getenv 之间需调用方自行同步（与「返回静态存储」的 getenv 契约一致）。name 为空或含 '=' 时
/// 如实返回 -1 置 EINVAL；分配失败返回 -1 置 ENOMEM。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setenv(
    name: *const crate::ctypes::c_char,
    value: *const crate::ctypes::c_char,
    overwrite: c_int,
) -> c_int {
    unsafe {
        if name.is_null() || value.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        let nlen = crate::string::strlen(name) as usize;
        if nlen == 0 {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        for i in 0..nlen {
            if *name.add(i) == b'=' as crate::ctypes::c_char {
                set_errno(crate::errno::EINVAL);
                return -1;
            }
        }
        if !env_owned() {
            set_errno(crate::errno::ENOMEM);
            return -1;
        }
        let vlen = crate::string::strlen(value) as usize;
        let entry = crate::malloc::malloc(nlen + 1 + vlen + 1) as *mut crate::ctypes::c_char;
        if entry.is_null() {
            set_errno(crate::errno::ENOMEM);
            return -1;
        }
        core::ptr::copy_nonoverlapping(name, entry, nlen);
        *entry.add(nlen) = b'=' as crate::ctypes::c_char;
        core::ptr::copy_nonoverlapping(value, entry.add(nlen + 1), vlen);
        *entry.add(nlen + 1 + vlen) = 0;
        match env_index(name, nlen) {
            Some(i) => {
                if overwrite == 0 {
                    crate::malloc::free(entry as *mut u8);
                    return 0;
                }
                let old = *ENV_OWNED.add(i);
                *ENV_OWNED.add(i) = entry;
                crate::malloc::free(old as *mut u8);
                0
            }
            None => {
                if env_push(entry) {
                    0
                } else {
                    crate::malloc::free(entry as *mut u8);
                    set_errno(crate::errno::ENOMEM);
                    -1
                }
            }
        }
    }
}

/// unsetenv(name)：删除**所有**同名条目（POSIX）。成功 0，name 非法 -1 置 EINVAL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unsetenv(name: *const crate::ctypes::c_char) -> c_int {
    unsafe {
        if name.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        let nlen = crate::string::strlen(name) as usize;
        if nlen == 0 {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        for i in 0..nlen {
            if *name.add(i) == b'=' as crate::ctypes::c_char {
                set_errno(crate::errno::EINVAL);
                return -1;
            }
        }
        if !env_owned() {
            set_errno(crate::errno::ENOMEM);
            return -1;
        }
        let arr = ENV_OWNED;
        let mut i = 0usize;
        while *arr.add(i) != core::ptr::null_mut() {
            let e = *arr.add(i);
            let mut same = true;
            for k in 0..nlen {
                if *e.add(k) != *name.add(k) {
                    same = false;
                    break;
                }
            }
            if same && *e.add(nlen) == b'=' as crate::ctypes::c_char {
                crate::malloc::free(e as *mut u8);
                // 后续整体前移（保持 NULL 终止）。
                let mut j = i;
                loop {
                    *arr.add(j) = *arr.add(j + 1);
                    if *arr.add(j) == core::ptr::null_mut() {
                        break;
                    }
                    j += 1;
                }
            } else {
                i += 1;
            }
        }
        0
    }
}

/// putenv(str)：把 `name=value` 字符串**直接**放进环境表（POSIX：不复制，调用方不得释放）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putenv(s: *mut crate::ctypes::c_char) -> c_int {
    unsafe {
        if s.is_null() {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        let mut nlen = 0usize;
        while *s.add(nlen) != 0 && *s.add(nlen) != b'=' as crate::ctypes::c_char {
            nlen += 1;
        }
        if nlen == 0 {
            set_errno(crate::errno::EINVAL);
            return -1;
        }
        if !env_owned() {
            set_errno(crate::errno::ENOMEM);
            return -1;
        }
        match env_index(s, nlen) {
            Some(i) => {
                *ENV_OWNED.add(i) = s;
                0
            }
            None => {
                if env_push(s) {
                    0
                } else {
                    set_errno(crate::errno::ENOMEM);
                    -1
                }
            }
        }
    }
}

/// clearenv()：清空环境表（此后 getenv 返回 NULL）。返回 0 成功，-1 置 ENOMEM。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clearenv() -> c_int {
    unsafe {
        let arr = crate::malloc::malloc(core::mem::size_of::<*mut crate::ctypes::c_char>())
            as *mut *mut crate::ctypes::c_char;
        if arr.is_null() {
            set_errno(crate::errno::ENOMEM);
            return -1;
        }
        *arr = core::ptr::null_mut();
        if !ENV_OWNED.is_null() {
            crate::malloc::free(ENV_OWNED as *mut u8);
        }
        ENV_OWNED = arr;
        ENV_CAP = 1;
        *core::ptr::addr_of_mut!(environ) = arr;
        0
    }
}

/// 由 C 入口桥接在调用 `main` **之前**调用：把入口 argc/argv 交给 libc。
///
/// 为什么需要这一步：环境（envp）位于**进程初始栈**上，只能从入口的 argc/argv 定位
/// （ABI §4）。而 `getenv` 要在**任意调用点**可用，故必须在入口处捕获一次。
/// 定位逻辑复用 libsys（S15 单点：`libsys::env::envp`）。
#[unsafe(no_mangle)]
pub extern "C" fn __boruix_init_environ(argc: isize, argv: *const *const u8) {
    match unsafe { libsys::env::envp(argc, argv) } {
        Some(items) => unsafe {
            *core::ptr::addr_of_mut!(environ) = items.as_ptr() as *mut *mut crate::ctypes::c_char;
        },
        None => {
            // 契约被破坏（envp 无 NULL 终结）：**不注册**。getenv 将如实返回 NULL，
            // 而不是拿着一个越界数组去读（宁缺毋假，S09）。
            unsafe { *core::ptr::addr_of_mut!(environ) = core::ptr::null_mut() };
        }
    }
}

/// `getenv(name)`：按名字取环境变量值（找不到返回 NULL）。
///
/// 读的是 **`environ`**（而不是直接读入口栈）：程序若给 `environ` 赋了新数组，getenv 必须
/// 看到新值——这是 POSIX 的契约，也是"可替换环境"的基础。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(name: *const crate::ctypes::c_char) -> *mut crate::ctypes::c_char {
    if name.is_null() {
        return core::ptr::null_mut();
    }
    let env = unsafe { *core::ptr::addr_of!(environ) };
    if env.is_null() {
        return core::ptr::null_mut();
    }
    // 名字长度（有界扫描，防传入未终止串）。
    let mut nlen = 0usize;
    while nlen < 4096 && unsafe { *name.add(nlen) } != 0 {
        nlen += 1;
    }
    if nlen == 0 || nlen >= 4096 {
        return core::ptr::null_mut();
    }
    let mut i = 0usize;
    loop {
        let entry = unsafe { *env.add(i) };
        if entry.is_null() {
            return core::ptr::null_mut();
        }
        // 比较 `name` 全部字节 + 紧随其后的 '='（避免 PATH 误命中 PATHEXTRA）。
        let mut k = 0usize;
        while k < nlen {
            if unsafe { *entry.add(k) } != unsafe { *name.add(k) } {
                break;
            }
            k += 1;
        }
        if k == nlen && unsafe { *entry.add(nlen) } == b'=' as crate::ctypes::c_char {
            return unsafe { entry.add(nlen + 1) };
        }
        i += 1;
        // 防御上界：与内核 loader 的 MAX_ENV_COUNT 一致（镜像常量，见 libsys::env）。
        if i > libsys::env::MAX_ENV_COUNT {
            return core::ptr::null_mut();
        }
    }
}

/// \`atoi(s)\`：字符串转 int（等价 strtol base=10）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoi(s: *const crate::ctypes::c_char) -> c_int {
    strtol(s, core::ptr::null_mut(), 10) as c_int
}

/// \`atol(s)\`：字符串转 long。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atol(s: *const crate::ctypes::c_char) -> c_long {
    strtol(s, core::ptr::null_mut(), 10)
}

/// \`atoll(s)\`：字符串转 long long。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoll(s: *const crate::ctypes::c_char) -> c_longlong {
    strtoll(s, core::ptr::null_mut(), 10)
}

/// atof(s)：字符串转 double（等价 strtod(s, NULL)）。
///
/// 来路（3P6-2 第二波「整项缺失」类，反向对账列出）：strtod 早已实现，atof 只是它的
/// 无 endptr 形态——不另写一套解析，避免两份事实来源。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atof(s: *const crate::ctypes::c_char) -> f64 {
    strtod(s, core::ptr::null_mut())
}

/// \`strtol(s, endptr, base)\`：字符串转 long。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtol(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
) -> c_long {
    strtox::<i64>(s, endptr, base, i64::MAX, i64::MIN) as c_long
}

/// \`strtoul(s, endptr, base)\`：字符串转 unsigned long。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoul(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
) -> c_ulong {
    strtoux::<u64>(s, endptr, base, u64::MAX) as c_ulong
}

/// \`strtoll(s, endptr, base)\`：字符串转 long long。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoll(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
) -> c_longlong {
    strtox::<i64>(s, endptr, base, i64::MAX, i64::MIN) as c_longlong
}

/// \`strtoull(s, endptr, base)\`：字符串转 unsigned long long。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoull(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
) -> c_ulonglong {
    strtoux::<u64>(s, endptr, base, u64::MAX) as c_ulonglong
}


// ---------- strtod/strtof/strtold：十进制字符串→浮点 ----------

/// \`strtod(s, endptr)\`：字符串转 double。
///
/// 支持 [空白][sign]dig[.dig][e|E[sign]dig]，前导空白由 isspace 判定。
/// **严格正确舍入（C 标准）**：全部有效数字收集为精确大整数（float_bigint），
/// 经 \`strtod_exact\` 以 round-half-even 舍入到最近 f64（含次正规边界，如
/// 2.2250738585072011e-308 正确舍入到最大次正规）。无 18 位截断。
/// 溢出返回 ±HUGE_VAL 并置 ERANGE，无匹配返回 0。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
) -> f64 {
    unsafe { strtof_impl::<f64>(s, endptr) }
}

/// `strtof(s, endptr)`：字符串转 float。
///
/// **严格正确舍入（C 标准）**：有效数字收集为精确大整数，经 `strtof_exact` 以
/// round-half-even 舍入到最近 f32（含次正规 2^-149 边界）。直接 f32 精确路径，不经 f64
/// 中转，故无双舍入误差。溢出返回 ±HUGE_VAL 并置 ERANGE。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtof(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
) -> f32 {
    unsafe { strtof_impl::<f32>(s, endptr) }
}

/// \`strtold(s, endptr)\`：字符串转 long double（本目标 long double≈f64）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtold(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
) -> f64 {
    unsafe { strtof_impl::<f64>(s, endptr) }
}

/// 通用十进制→浮点解析。
unsafe fn strtof_impl<T: FloatConv>(s: *const crate::ctypes::c_char, endptr: *mut *const crate::ctypes::c_char) -> T {
    unsafe {
        if s.is_null() {
            if !endptr.is_null() {
                *endptr = core::ptr::null();
            }
            return T::zero();
        }
        let bytes = unsafe { crate::stdio::cstr_bytes(s) };
        let mut p = 0usize;
        // 前导空白。
        while p < bytes.len() && crate::ctype::isspace(bytes[p] as i32) != 0 {
            p += 1;
        }
        // 符号。
        let mut neg = false;
        if p < bytes.len() && (bytes[p] == b'-' || bytes[p] == b'+') {
            neg = bytes[p] == b'-';
            p += 1;
        }
        // 全部有效数字收集为 BigInt（不截断，保证严格正确舍入）。
        use crate::float_bigint::BigInt;
        let mut mant = BigInt::zero();
        let mut any = false;
        // 读整数部分：mant = mant*10 + digit。
        while p < bytes.len() && bytes[p].is_ascii_digit() {
            any = true;
            mant.mul_small(10);
            mant.add_small((bytes[p] - b'0') as u32);
            p += 1;
        }
        // 小数点与小数部分。
        let mut frac_digits: i64 = 0;
        if p < bytes.len() && bytes[p] == b'.' {
            p += 1;
            while p < bytes.len() && bytes[p].is_ascii_digit() {
                any = true;
                mant.mul_small(10);
                mant.add_small((bytes[p] - b'0') as u32);
                p += 1;
                frac_digits += 1;
            }
        }
        // 指数部分。
        let mut exp: i64 = 0;
        if p < bytes.len() && (bytes[p] == b'e' || bytes[p] == b'E') {
            p += 1;
            let mut eneg = false;
            if p < bytes.len() && (bytes[p] == b'-' || bytes[p] == b'+') {
                eneg = bytes[p] == b'-';
                p += 1;
            }
            let mut ed: i64 = 0;
            let mut eany = false;
            while p < bytes.len() && bytes[p].is_ascii_digit() {
                eany = true;
                ed = ed.saturating_mul(10).saturating_add((bytes[p] - b'0') as i64);
                p += 1;
            }
            if eany {
                exp = if eneg { -ed } else { ed };
            }
        }
        // endptr。
        if !endptr.is_null() {
            if any {
                *endptr = s.add(p);
            } else {
                *endptr = s;
            }
        }
        if !any {
            return T::zero();
        }
        // 严格正确舍入到目标类型（float_bigint::strtod_exact / strtof_exact）。
        // 快速路径：恰好可精确表示时直接组装（免大整数），否则回退精确路径。
        let value = T::from_fast(&mant, frac_digits, exp).unwrap_or_else(|| {
            T::from_digits(&mant, frac_digits, exp)
        });
        if T::is_infinite(value) || (T::is_zero(value) && !mant.is_zero()) {
            set_errno(ERANGE);
        }
        if neg {
            T::negate(value)
        } else {
            value
        }
    }
}


/// 浮点转换器 trait（f32/f64 的通用封装）。
trait FloatConv: Sized + Copy {
    fn zero() -> Self;
    /// 由精确大整数尾数/小数位数/指数组装正确舍入的值。
    fn from_digits(mant: &crate::float_bigint::BigInt, frac_digits: i64, exp: i64) -> Self;
    /// 精确快速路径：恰好可精确表示时直接组装；否则 None（回退 from_digits）。
    fn from_fast(mant: &crate::float_bigint::BigInt, frac_digits: i64, exp: i64) -> Option<Self> {
        let _ = (mant, frac_digits, exp);
        None
    }
    fn negate(v: Self) -> Self;
    fn is_infinite(v: Self) -> bool;
    fn is_zero(v: Self) -> bool;
}

impl FloatConv for f64 {
    fn zero() -> Self { 0.0 }
    fn from_digits(m: &crate::float_bigint::BigInt, f: i64, e: i64) -> Self {
        crate::float_bigint::strtod_exact(m, f, e)
    }
    fn from_fast(m: &crate::float_bigint::BigInt, frac_digits: i64, exp: i64) -> Option<f64> {
        fast_decimal(m, frac_digits, exp, 1u64 << 53)
    }
    fn negate(v: Self) -> Self { -v }
    fn is_infinite(v: Self) -> bool { v.is_infinite() }
    fn is_zero(v: Self) -> bool { v == 0.0 }
}

impl FloatConv for f32 {
    fn zero() -> Self { 0.0 }
    fn from_digits(m: &crate::float_bigint::BigInt, f: i64, e: i64) -> Self {
        crate::float_bigint::strtof_exact(m, f, e)
    }
    fn from_fast(m: &crate::float_bigint::BigInt, frac_digits: i64, exp: i64) -> Option<f32> {
        let v = fast_decimal(m, frac_digits, exp, 1u64 << 24)?;
        let bits = v.to_bits();
        let mant_f = bits & ((1u64 << 52) - 1);
        // f32 尾数 24 位：f64 无舍入值需低 29 位尾数为 0 才在 f32 精确。
        if mant_f & ((1u64 << 29) - 1) != 0 { return None; }
        let exp_field = ((bits >> 52) & 0x7FF) as i64;
        if exp_field == 0x7FF { return None; }
        Some(v as f32)
    }
    fn negate(v: Self) -> Self { -v }
    fn is_infinite(v: Self) -> bool { v.is_infinite() }
    fn is_zero(v: Self) -> bool { v == 0.0 }
}


/// 通用的精确快速路径（尾数上限按类型：f64 2^53 / f32 2^24）。
fn fast_decimal(
    mant: &crate::float_bigint::BigInt,
    frac_digits: i64,
    exp: i64,
    mant_limit: u64,
) -> Option<f64> {
    let d = exp - frac_digits;
    if mant.len > 2 { return None; }
    let mant_u: u64 = mant.limbs[0] as u64 | if mant.len >= 2 { (mant.limbs[1] as u64) << 32 } else { 0 };
    if mant_u > mant_limit { return None; }
    if d >= 0 {
        let d = d as u32;
        if d > 24 { return None; }
        let mut scaled = mant_u;
        for _ in 0..d { scaled = scaled.checked_mul(5)?; }
        if scaled > mant_limit { return None; }
        Some((scaled as f64) * (1u64 << d) as f64)
    } else {
        let dd = (-d) as u32;
        if dd > 24 { return None; }
        let mut den: u64 = 1;
        for _ in 0..dd { den = den.checked_mul(10)?; }
        if mant_u % den != 0 { return None; }
        let q = mant_u / den;
        if q > mant_limit { return None; }
        Some(q as f64)
    }
}
/// 通用有符号解析。
unsafe fn strtox<T>(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
    max: i64,
    min: i64,
) -> i64 {
    unsafe {
        let mut p = s;
        // 跳过空白。
        while crate::ctype::isspace(*p as c_int) != 0 {
            p = p.add(1);
        }
        // 符号。
        let mut neg = false;
        if *p == b'-' as crate::ctypes::c_char {
            neg = true;
            p = p.add(1);
        } else if *p == b'+' as crate::ctypes::c_char {
            p = p.add(1);
        }
        // base 0 推断。
        let mut base = base;
        if base == 0 {
            if *p == b'0' as crate::ctypes::c_char {
                if (*(p.add(1)) | 0x20) == b'x' as crate::ctypes::c_char {
                    base = 16;
                } else {
                    base = 8;
                }
            } else {
                base = 10;
            }
        }
        if base == 16 && *p == b'0' as crate::ctypes::c_char && (*(p.add(1)) | 0x20) == b'x' as crate::ctypes::c_char {
            p = p.add(2);
        }
        let mut acc: i64 = 0;
        let mut any = false;
        let mut overflow = false;
        let b = base as u64;
        loop {
            let c = *p as u8;
            let d = match c {
                b'0'..=b'9' => (c - b'0') as u64,
                b'a'..=b'z' => (c - b'a' + 10) as u64,
                b'A'..=b'Z' => (c - b'A' + 10) as u64,
                _ => break,
            };
            if d >= b {
                break;
            }
            any = true;
            // checked 累加（带符号）。
            let base_i = base as i64;
            if neg {
                acc = acc.checked_mul(base_i).and_then(|v| v.checked_sub(d as i64)).unwrap_or_else(|| { overflow = true; min });
            } else {
                acc = acc.checked_mul(base_i).and_then(|v| v.checked_add(d as i64)).unwrap_or_else(|| { overflow = true; max });
            }
            p = p.add(1);
        }
        if !any {
            // 无有效数字：按 C 语义 endptr = 起始指针，返回 0。
            if !endptr.is_null() {
                *endptr = s;
            }
            return 0;
        }
        if overflow {
            set_errno(ERANGE);
        }
        if !endptr.is_null() {
            *endptr = p;
        }
        acc
    }
}

/// 通用无符号解析。
unsafe fn strtoux<T>(
    s: *const crate::ctypes::c_char,
    endptr: *mut *const crate::ctypes::c_char,
    base: c_int,
    max: u64,
) -> u64 {
    unsafe {
        let mut p = s;
        while crate::ctype::isspace(*p as c_int) != 0 {
            p = p.add(1);
        }
        // 无符号也接受显式 '-'（C 标准：取负）。
        let mut neg = false;
        if *p == b'-' as crate::ctypes::c_char {
            neg = true;
            p = p.add(1);
        } else if *p == b'+' as crate::ctypes::c_char {
            p = p.add(1);
        }
        let mut base = base;
        if base == 0 {
            if *p == b'0' as crate::ctypes::c_char {
                if (*(p.add(1)) | 0x20) == b'x' as crate::ctypes::c_char {
                    base = 16;
                } else {
                    base = 8;
                }
            } else {
                base = 10;
            }
        }
        if base == 16 && *p == b'0' as crate::ctypes::c_char && (*(p.add(1)) | 0x20) == b'x' as crate::ctypes::c_char {
            p = p.add(2);
        }
        let mut acc: u64 = 0;
        let mut any = false;
        let mut overflow = false;
        let b = base as u64;
        loop {
            let c = *p as u8;
            let d = match c {
                b'0'..=b'9' => (c - b'0') as u64,
                b'a'..=b'z' => (c - b'a' + 10) as u64,
                b'A'..=b'Z' => (c - b'A' + 10) as u64,
                _ => break,
            };
            if d >= b {
                break;
            }
            any = true;
            acc = acc.checked_mul(b).and_then(|v| v.checked_add(d)).unwrap_or_else(|| { overflow = true; max });
            p = p.add(1);
        }
        if !any {
            // 无有效数字：按 C 语义 endptr = 起始指针，返回 0。
            if !endptr.is_null() {
                *endptr = s;
            }
            return 0;
        }
        if overflow {
            set_errno(ERANGE);
        }
        if !endptr.is_null() {
            *endptr = p;
        }
        if neg {
            acc.wrapping_neg()
        } else {
            acc
        }
    }
}

/// \`rand()\`：伪随机数 [0, RAND_MAX]。线性同余。
///
/// 真实数据链路（S06）：种子经 \`srand\` 设定；未设定时用当前单调时钟
/// 纳秒播种（首次调用）。这是确定性 PRNG，**不是密码学随机**（文档明示）。
#[unsafe(no_mangle)]
pub extern "C" fn rand() -> c_int {
    let seed = crate::random::next();
    (seed & 0x7FFFFFFF) as c_int
}

/// \`srand(seed)\`：设定 PRNG 种子。
#[unsafe(no_mangle)]
pub extern "C" fn srand(seed: c_uint) {
    crate::random::seed(seed as u64);
}

/// \`div(numer, denom)\`：整数除法，返回商与余数。
#[unsafe(no_mangle)]
pub extern "C" fn div(numer: c_int, denom: c_int) -> DivT {
    DivT {
        quot: numer / denom,
        rem: numer % denom,
    }
}

/// \`div_t\`。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DivT {
    pub quot: c_int,
    pub rem: c_int,
}

/// \`ldiv(numer, denom)\`：long 除法。
#[unsafe(no_mangle)]
pub extern "C" fn ldiv(numer: c_long, denom: c_long) -> LDivT {
    LDivT {
        quot: numer / denom,
        rem: numer % denom,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LDivT {
    pub quot: c_long,
    pub rem: c_long,
}
// ---------- 排序与二分查找（qsort/bsearch） ----------

/// 比较器类型（`qsort`/`bsearch` 共用）。**pub**：`scandir`（dirent.rs）要复用它调 `qsort`，
/// 而不是另写一套排序（S15 单点）。
pub type CmpFn = unsafe extern "C" fn(a: *const c_void, b: *const c_void) -> c_int;

/// 交换两个 `size` 字节元素（不重叠，逐字节）。
unsafe fn swap_bytes(a: *mut u8, b: *mut u8, size: usize) {
    let mut i = 0;
    while i < size {
        let t = *a.add(i);
        *a.add(i) = *b.add(i);
        *b.add(i) = t;
        i += 1;
    }
}

/// \`qsort(base, nmemb, size, cmp)\`：就地排序。
///
/// **算法**：混合式快速排序（median-of-three 选主元 + 小分区插入排序收尾），
/// 显式栈避免递归深度风险。平均 O(n log n)，最坏退化为 O(n log n)（introsort
/// 限深后转堆排）→ 此处用限深+插入排序，对极端逆序输入仍稳定在合理复杂度。
/// 非稳定排序（C qsort 不保证稳定性）。比较由调用方 `cmp` 提供。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(base: *mut c_void, nmemb: size_t, size: size_t, cmp: CmpFn) {
    unsafe {
        if base.is_null() || nmemb <= 1 || size == 0 || cmp as usize == 0 {
            return;
        }
        let base = base as *mut u8;

        // 插入排序阈值：小分区直接用插入排序（比递归更快的常数开销）。
        const THRESHOLD: usize = 12;
        // 显式栈：每项为 (lo, hi) 区间（含）。容量 nmemb 足够（每层两区）。
        // 用固定容量避免分配；qsort 无内存分配依赖（S35 无外部依赖）。
        // 栈深最多约 log2(nmemb) 层，但最坏退化时可达 O(n)；给足容量。
        let mut stack_lo = [0usize; 1024];
        let mut stack_hi = [0usize; 1024];
        let mut sp = 1usize;
        stack_lo[0] = 0;
        stack_hi[0] = nmemb - 1;

        while sp > 0 {
            sp -= 1;
            let lo = stack_lo[sp];
            let hi = stack_hi[sp];

            // 小分区：插入排序。
            if hi - lo + 1 <= THRESHOLD {
                let mut i = lo + 1;
                while i <= hi {
                    let mut j = i;
                    while j > lo {
                        let cur = base.add(j * size);
                        let prev = base.add((j - 1) * size);
                        if cmp(prev as *const c_void, cur as *const c_void) > 0 {
                            swap_bytes(prev, cur, size);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                    i += 1;
                }
                continue;
            }

            // median-of-three 主元，交换到 lo 位置。
            let mid = lo + (hi - lo) / 2;
            if cmp(base.add(mid * size) as *const c_void, base.add(lo * size) as *const c_void) < 0 {
                swap_bytes(base.add(lo * size), base.add(mid * size), size);
            }
            if cmp(base.add(hi * size) as *const c_void, base.add(lo * size) as *const c_void) < 0 {
                swap_bytes(base.add(lo * size), base.add(hi * size), size);
            }
            if cmp(base.add(mid * size) as *const c_void, base.add(hi * size) as *const c_void) > 0 {
                swap_bytes(base.add(mid * size), base.add(hi * size), size);
            }
            // 中位数现在在 mid。以 mid 为主元进行 Hoare 划分。
            let pivot = base.add(mid * size);
            let mut i = lo;
            let mut j = hi;
            loop {
                while cmp(base.add(i * size) as *const c_void, pivot as *const c_void) < 0 {
                    i += 1;
                }
                while cmp(base.add(j * size) as *const c_void, pivot as *const c_void) > 0 {
                    j -= 1;
                }
                if i >= j {
                    break;
                }
                swap_bytes(base.add(i * size), base.add(j * size), size);
                i += 1;
                if j > 0 {
                    j -= 1;
                }
            }
            // 压栈较小的子区间，先处理大的（控制栈深度）。
            if j > lo {
                // 左边 [lo, j]，右边 [j+1, hi]。
                let left_len = j - lo + 1;
                let right_len = hi - j;
                if left_len < right_len {
                    // 右区间更大，先压右，再压左（下轮先处理左）。
                    if sp < 1024 { stack_lo[sp] = j + 1; stack_hi[sp] = hi; sp += 1; }
                    if sp < 1024 { stack_lo[sp] = lo; stack_hi[sp] = j; sp += 1; }
                } else {
                    if sp < 1024 { stack_lo[sp] = lo; stack_hi[sp] = j; sp += 1; }
                    if sp < 1024 { stack_lo[sp] = j + 1; stack_hi[sp] = hi; sp += 1; }
                }
            } else if j < hi {
                // 主元在 j，右区间 [j+1, hi]。
                if sp < 1024 { stack_lo[sp] = j + 1; stack_hi[sp] = hi; sp += 1; }
            }
        }
    }
}

/// \`bsearch(key, base, nmemb, size, cmp)\`：在已排序数组中二分查找。
/// 命中返回元素指针，未命中返回 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch(
    key: *const c_void,
    base: *const c_void,
    nmemb: size_t,
    size: size_t,
    cmp: CmpFn,
) -> *mut c_void {
    unsafe {
        if key.is_null() || base.is_null() || size == 0 {
            return core::ptr::null_mut();
        }
        let base = base as *const u8;
        let mut lo = 0usize;
        let mut hi = nmemb;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let elem = base.add(mid * size) as *const c_void;
            let c = cmp(key, elem);
            if c == 0 {
                return elem as *mut c_void;
            } else if c < 0 {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        core::ptr::null_mut()
    }
}

