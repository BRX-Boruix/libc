//! `qsort` 的**纯计算核心**（无 syscall、无 crate 依赖）——为的是能在**宿主上用参考排序对照验证**。
//!
//! ## 为什么单独成文件（与 `math_core.rs` 同一条理由）
//!
//! `stdlib.rs` 的 `qsort` 依赖 libsys，宿主上链不起来；而排序算法的正确性（尤其"越界"这类）
//! 必须能被**大量随机输入**逐点对照——那是唯一能在秒级抓出问题的手段。
//! 故算法放这里（只用 `core::ffi`），导出层在 `stdlib.rs`。
//!
//! ## 这里修掉的真 bug（2026-10，在 Boruix 内跑 cc1 时暴露）
//!
//! 原实现用 **Hoare 划分**，`pivot` 是**指向数组内部的指针**。第一次 `swap` 之后 pivot 元素
//! 就被搬走了，于是：
//!   ① 内层 `while cmp(a[i], pivot) < 0 { i += 1 }` **没有边界检查**——pivot 值一旦是区间最大，
//!      `i` 会一路扫出数组末尾 ⇒ **越界读** ⇒ 段错误；
//!   ② 即使不越界，划分也不再围绕真正的 pivot，结果可能不有序。
//! 实测形态：`cc1 cc1-smoke.c` 在 `during GIMPLE pass: cfg` 段错误（GCC 的 CFG pass 会调 qsort）。
//!
//! 现改为 **Lomuto 划分**：主元换到 `hi` 后就**不再移动**（直到最后一步），故全程无越界可能。
//! 另：每轮**先处理较小的一侧、把较大的一侧压栈**，栈深恒 ≤ log2(n)（原实现最坏可到 O(n)，
//! 且 `sp` 满时**静默丢弃**区间 ⇒ 悄悄返回未排序结果）。

use core::ffi::{c_int, c_void};

/// 比较函数类型（与 C 的 `int (*)(const void *, const void *)` 一致）。
pub type QCmp = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

/// 交换两个 `size` 字节的元素（`a == b` 时无操作）。
#[inline]
unsafe fn swap_bytes(a: *mut u8, b: *mut u8, size: usize) {
    if a == b {
        return;
    }
    let mut i = 0usize;
    while i < size {
        let t = *a.add(i);
        *a.add(i) = *b.add(i);
        *b.add(i) = t;
        i += 1;
    }
}

/// 就地排序 `nmemb` 个元素，每个 `size` 字节，比较用 `cmp`。
///
/// **正确性论证（关键点）**：
/// - 主元被换到 `hi` 后，Lomuto 扫描只访问 `[lo, hi-1]`，主元位置 `hi` 全程不动
///   ⇒ **任何比较都不会读到区间外**；
/// - 扫描结束后 `swap(i, hi)` 把主元落位到 `i`，于是 `[lo, i-1] < pivot` 且 `[i+1, hi] >= pivot`；
/// - 两个子区间**严格小于**原区间（`i` 至少把主元分出去）⇒ 必然终止；
/// - 先处理小的一侧、大的压栈 ⇒ 栈深 ≤ ⌊log2(n)⌋ + 1。
pub unsafe fn core_qsort(base: *mut u8, nmemb: usize, size: usize, cmp: QCmp) {
    if base.is_null() || nmemb <= 1 || size == 0 {
        return;
    }
    // 插入排序阈值：小分区直接插入排序，常数开销更低。
    const THRESHOLD: usize = 12;
    // 显式栈。**先处理小侧**保证栈深 ≤ log2(n)，1024 对任何 n 都富余（n 是 usize，log2 ≤ 64）。
    let mut stack_lo = [0usize; 64];
    let mut stack_hi = [0usize; 64];
    let mut sp = 1usize;
    stack_lo[0] = 0;
    stack_hi[0] = nmemb - 1;

    #[inline]
    unsafe fn elem(base: *mut u8, idx: usize, size: usize) -> *mut u8 {
        unsafe { base.add(idx * size) }
    }

    while sp > 0 {
        sp -= 1;
        let mut lo = stack_lo[sp];
        let mut hi = stack_hi[sp];
        loop {
            if hi <= lo {
                break;
            }
            if hi - lo + 1 <= THRESHOLD {
                // 插入排序 [lo, hi]。
                let mut i = lo + 1;
                while i <= hi {
                    let mut j = i;
                    while j > lo {
                        let cur = elem(base, j, size);
                        let prev = elem(base, j - 1, size);
                        if cmp(prev as *const c_void, cur as *const c_void) > 0 {
                            swap_bytes(prev, cur, size);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                    i += 1;
                }
                break;
            }
            // median-of-three：把 lo/mid/hi 三者排序，然后**把中位数换到 hi 当主元**。
            let mid = lo + (hi - lo) / 2;
            if cmp(elem(base, mid, size) as *const c_void, elem(base, lo, size) as *const c_void) < 0 {
                swap_bytes(elem(base, lo, size), elem(base, mid, size), size);
            }
            if cmp(elem(base, hi, size) as *const c_void, elem(base, lo, size) as *const c_void) < 0 {
                swap_bytes(elem(base, lo, size), elem(base, hi, size), size);
            }
            if cmp(elem(base, hi, size) as *const c_void, elem(base, mid, size) as *const c_void) < 0 {
                swap_bytes(elem(base, mid, size), elem(base, hi, size), size);
            }
            // 此时 a[lo] <= a[mid] <= a[hi]；把中位数 a[mid] 换到 hi 作主元。
            swap_bytes(elem(base, mid, size), elem(base, hi, size), size);
            let pivot = elem(base, hi, size) as *const c_void;

            // **3 路划分（Dutch national flag）**：分成 <pivot / ==pivot / >pivot 三段。
            // 主元在 `hi` 全程不动——循环条件是 `i < gt`，而 `gt` 从 `hi` 起只减不加。
            //
            // **为什么必须 3 路**（2026-10 实测的根因）：2 路 Lomuto 在**大量相等键**上
            // 退化 O(n²)。宿主实测（n=160k）：全相等键 **6.469 秒**、2 个键 3.254 秒，
            // 而随机输入只要 **13.8 毫秒**（慢 470 倍）。ELF 的**局部符号名字为空**，
            // `tcc` 的 `sort_syms` 正是这种输入 ⇒ 系统内 `tcc g.o -o g` 的链接一步
            // 实测 **176 秒**，瓶颈就在这里。
            let pivot = elem(base, hi, size) as *const c_void;
            let mut lt = lo; // [lo, lt) 全 < pivot
            let mut i = lo;  // [lt, i) 全 == pivot
            let mut gt = hi; // [gt, hi] 全 > pivot，且 **a[hi] 本身不动**
            while i < gt {
                let c = cmp(elem(base, i, size) as *const c_void, pivot);
                if c < 0 {
                    if lt != i {
                        swap_bytes(elem(base, lt, size), elem(base, i, size), size);
                    }
                    lt += 1;
                    i += 1;
                } else if c > 0 {
                    gt -= 1;
                    swap_bytes(elem(base, i, size), elem(base, gt, size), size);
                    // 换进来的元素**尚未检查**，故 i 不前进。
                } else {
                    i += 1;
                }
            }
            // **把主元从 hi 归位到 gt**：循环结束时 [lt, gt) == pivot 而 [gt, hi-1] > pivot，
            // 但 a[hi] 仍是 pivot（== pivot）——它排在那些 > pivot 的元素**之后**，是错的。
            // 与 a[gt] 交换后：[lt, gt] == pivot、[gt+1, hi] > pivot，两段才各自连续。
            // （首版漏了这一步，宿主对照立刻报"随机/已升序 有序=false"。）
            if gt != hi {
                swap_bytes(elem(base, gt, size), elem(base, hi, size), size);
            }
            // 现在：[lo, lt) < pivot；[lt, gt] == pivot（**整段跳过，不递归**）；[gt+1, hi] > pivot。
            // 等键段不递归 ⇒ 全相等输入是 O(n)（这正是修掉的那个退化）。
            let l_n = lt - lo;
            let r_n = hi - gt;
            // **先处理较小的一侧**，把较大的一侧压栈（栈深 ≤ log2 n）。
            if l_n < r_n {
                if r_n > 0 {
                    stack_lo[sp] = gt + 1;
                    stack_hi[sp] = hi;
                    sp += 1;
                }
                if l_n > 0 {
                    hi = lt - 1;
                } else {
                    break;
                }
            } else {
                if l_n > 0 {
                    stack_lo[sp] = lo;
                    stack_hi[sp] = lt - 1;
                    sp += 1;
                }
                if r_n > 0 {
                    lo = gt + 1;
                } else {
                    break;
                }
            }
        }
    }
}
