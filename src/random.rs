//! 伪随机数生成器（供 rand/srand 使用）。
//!
//! 确定性 xorshift64* 型 PRNG，经全局原子种子状态。\`rand\` 未调用 \`srand\`
//! 时，首次调用用单调时钟纳秒播种——这是**确定性 PRNG**，非密码学安全
//! （文档明示；若要密码学熵，应接内核 \`/devices/random\` 或 RDRAND）。

use core::sync::atomic::{AtomicU64, Ordering};

/// 全局 PRNG 状态（xorshift64*）。
static STATE: AtomicU64 = AtomicU64::new(0);
/// 是否已播种。
static SEEDED: AtomicU64 = AtomicU64::new(0);

/// 设置种子。
pub fn seed(s: u64) {
    // 避免全 0（xorshift 全 0 恒为 0）。
    let s = if s == 0 { 0x9E3779B97F4A7C15 } else { s };
    STATE.store(s, Ordering::Relaxed);
    SEEDED.store(1, Ordering::Relaxed);
}

/// 取下一个伪随机 u64。
pub fn next() -> u64 {
    // 未播种则用单调时钟播种一次。
    if SEEDED.load(Ordering::Relaxed) == 0 {
        let t = libsys::info(libsys::nr::INFO_BOOT_MS).unwrap_or(0);
        seed(t.wrapping_mul(0x9E3779B97F4A7C15) ^ 0x1234567890ABCDEF);
    }
    let mut x = STATE.load(Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    STATE.store(x, Ordering::Relaxed);
    x.wrapping_mul(0x2545F4914F6CDD1D)
}
