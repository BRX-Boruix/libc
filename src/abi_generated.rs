//! **自动生成，请勿手改。**
//!
//! 源：tools/abi/constants.json
//! 生成：python tools/abi/generate.py
//! 门禁：python tools/abi/generate.py --check（已登记进 tools/checks/run_all.py）
//!
//! 手改本文件会被门禁发现；要改数值请改 JSON 再重新生成。

use crate::ctypes::{c_int, c_long};

// ---------- 跨 crate 数值（同一数值只在这里写一次） ----------

pub const PAGE_SIZE: c_int = 4096;
pub const PATH_MAX: c_long = 4096;
pub const NAME_MAX: c_long = 255;
pub const PIPE_BUF: c_long = 4096;

// ---------- 默认 PATH（与 shell 的 DEFAULT_PATH 同源） ----------

pub const DEFAULT_PATH_C: &[u8] = b"/programs:/volumes/BORUIX_DATA/3p\0";
