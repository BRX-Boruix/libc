//! SHA-256（FIPS 180-4）——A2-7 认证所需的无依赖哈希，**单点实现**（S13）。
//!
//! ## 为何自研而不是引依赖
//!
//! 本仓无任何密码学依赖，且用户态程序运行在 `no_std` 裸机目标上。ADR-041 §1.2.4 已就此
//! 成文：引入一个**未经评审的自定义构造**比"明确声明强度边界"更危险。本实现的选择是
//! **严格照抄 FIPS 180-4 的标准算法**（不含任何自创变形），并用**官方测试向量**验证，
//! 使其正确性可被独立判定——而不是"AES 类自创轮函数 + 看起来对"。
//!
//! ## 强度边界（ADR-041 §1.2.4，S39 如实声明）
//!
//! 单轮 `SHA-256(salt || password)` **不是**口令哈希的现代做法（应为 bcrypt/scrypt/Argon2
//! 等**慢哈希**）。其安全性**完全依赖**"攻击者拿不到 salt+hash"这一前提——即 ADR-041
//! §1.2.1 实测的 shadow 文件分离（`0600` + 属主 root-only）。**一旦 shadow 泄漏，
//! 单轮 SHA-256 对弱口令几乎不提供保护。** 此处不得给出任何更强的暗示。
//!
//! ## 实现纪律
//!
//! - **常量单点**：K 表与初始 H 值照 FIPS 180-4 给出，不做化简；
//! - **不做短路优化**：哪怕"看起来等价"的位运算合并也不做——本模块的正确性来自
//!   "与标准逐字一致"，任何改写都会使测试向量的意义下降；
//! - **无 panic 路径**：所有缓冲长度固定，索引来自定长循环，不依赖调用方输入。

/// SHA-256 输出字节数。
pub const SHA256_DIGEST_LEN: usize = 32;
/// SHA-256 内部块字节数。
const BLOCK_LEN: usize = 64;

/// 轮常量 K（FIPS 180-4 §4.2.2）：前 64 个素数立方根小数部分的前 32 位。
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 初始哈希值 H(0)（FIPS 180-4 §5.3.3）：前 8 个素数平方根小数部分的前 32 位。
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 流式上下文（支持分块喂入，避免为长输入分配整块缓冲）。
#[derive(Clone)]
pub struct Sha256 {
    /// 中间哈希状态（8 × u32）。
    h: [u32; 8],
    /// 未满一个块的部分数据。
    buf: [u8; BLOCK_LEN],
    /// `buf` 中已填充字节数（恒 < BLOCK_LEN）。
    buf_len: usize,
    /// 已处理的**完整块**数（用于总长度计算，避免 usize 溢出）。
    blocks: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// 新建空上下文。
    pub const fn new() -> Self {
        Self {
            h: H0,
            buf: [0u8; BLOCK_LEN],
            buf_len: 0,
            blocks: 0,
        }
    }

    /// 喂入任意长度数据（可多次调用）。
    pub fn update(&mut self, mut data: &[u8]) {
        // 先补满当前块。
        if self.buf_len > 0 {
            let need = BLOCK_LEN - self.buf_len;
            let take = if data.len() < need { data.len() } else { need };
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == BLOCK_LEN {
                let block = self.buf;
                self.compress(&block);
                self.blocks += 1;
                self.buf_len = 0;
            }
        }
        // 整块处理。
        while data.len() >= BLOCK_LEN {
            let mut block = [0u8; BLOCK_LEN];
            block.copy_from_slice(&data[..BLOCK_LEN]);
            self.compress(&block);
            self.blocks += 1;
            data = &data[BLOCK_LEN..];
        }
        // 余下不足一块。
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    /// 完成计算并返回 32 字节摘要。
    pub fn finish(mut self) -> [u8; SHA256_DIGEST_LEN] {
        // 总消息长度（比特）：blocks*64 + buf_len，乘 8 得比特数。
        let total_bits: u64 = self.blocks * (BLOCK_LEN as u64) * 8 + (self.buf_len as u64) * 8;
        // 填充 0x80，随后补 0 到 56 mod 64，最后 8 字节大端长度。
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > BLOCK_LEN - 8 {
            // 本块放不下长度字段：补 0 满块，处理后再开新块。
            while self.buf_len < BLOCK_LEN {
                self.buf[self.buf_len] = 0;
                self.buf_len += 1;
            }
            let block = self.buf;
            self.compress(&block);
            self.buf = [0u8; BLOCK_LEN];
            self.buf_len = 0;
        }
        while self.buf_len < BLOCK_LEN - 8 {
            self.buf[self.buf_len] = 0;
            self.buf_len += 1;
        }
        self.buf[BLOCK_LEN - 8..].copy_from_slice(&total_bits.to_be_bytes());
        let block = self.buf;
        self.compress(&block);

        let mut out = [0u8; SHA256_DIGEST_LEN];
        for (i, w) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }

    /// 压缩函数（FIPS 180-4 §6.2.2）：处理一个 64 字节块。
    fn compress(&mut self, block: &[u8; BLOCK_LEN]) {
        // 消息调度 W[0..64]。
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        // 8 个工作变量。
        let mut a = self.h[0];
        let mut b = self.h[1];
        let mut c = self.h[2];
        let mut d = self.h[3];
        let mut e = self.h[4];
        let mut f = self.h[5];
        let mut g = self.h[6];
        let mut hh = self.h[7];
        for i in 0..64 {
            let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(big_s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = big_s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(hh);
    }
}

/// 一次性计算 `data` 的 SHA-256。
pub fn sha256(data: &[u8]) -> [u8; SHA256_DIGEST_LEN] {
    let mut c = Sha256::new();
    c.update(data);
    c.finish()
}

/// 十六进制小写编码（32 字节 → 64 字符）。
pub fn to_hex(bytes: &[u8], out: &mut [u8]) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut n = 0usize;
    for &b in bytes {
        if n + 2 > out.len() {
            break;
        }
        out[n] = HEX[(b >> 4) as usize];
        out[n + 1] = HEX[(b & 0x0f) as usize];
        n += 2;
    }
    n
}

// ---------------------------------------------------------------------------
// 已知答案测试（FIPS 180-4 / NIST 标准向量）。
//
// **关于这些测试的执行方式（S39 如实说明）**：libc 在 host 上无法运行 `cargo test`
// （裸机目标：无全局分配器、无 `#[panic_handler]`、不支持 unwinding）——此为既有
// 状况，非本模块引入。故这些向量**在 host 上不执行**，其真实校验发生在目标机：
// `libccheck` 内建的 sha256 段会把这些向量的**判定搬到真实内核上执行**并逐条断言。
// 保留于此的意义是：算法意图与预期值在源码内可读、可对照 FIPS 文本复查（可审计性）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod fips_tests {
    use super::*;

    fn hex(d: &[u8]) -> alloc::string::String {
        let mut out = [0u8; 64];
        let n = to_hex(d, &mut out);
        alloc::string::String::from(core::str::from_utf8(&out[..n]).unwrap_or(""))
    }

    #[test]
    fn empty_string() {
        // FIPS 180-4 示例：SHA-256("")
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn abc() {
        // FIPS 180-4 示例：SHA-256("abc")
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn two_block_message() {
        // FIPS 180-4 示例：448 比特（56 字节）消息
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn padding_boundary_55_56_64() {
        // **边界最关键**：55 字节（长度字段恰好落在本块内）、56 字节（需额外一块）、
        // 64 字节（恰好一个整块）。这三例专门覆盖 finish() 的填充分支。
        assert_eq!(
            hex(&sha256(&[b'a'; 55])),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 56])),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 64])),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    #[test]
    fn streaming_equals_one_shot() {
        // 分块喂入必须与一次喂入结果相同（覆盖 update() 的块拼接路径）。
        let data: alloc::vec::Vec<u8> = (0u8..=255).collect();
        let one = sha256(&data);
        let mut c = Sha256::new();
        for chunk in data.chunks(7) {
            c.update(chunk);
        }
        assert_eq!(one, c.finish());
    }
}
