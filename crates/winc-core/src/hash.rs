//! 64 位 FNV-1a 哈希：结果只取决于输入的字节，每次运行、每台机器上都相同。
//!
//! `std` 的哈希器每次运行使用不同的种子，不能用于要写到磁盘上、或要跨进程比较的值（备份目录
//! 名、仓库配置的指纹）。哈希结果一旦写到磁盘上就不能再变：改动算法会使已有的备份目录找不到、
//! 已信任的仓库需要重新确认。

/// FNV-1a 的初始值（64 位的 offset basis，无单位）。
const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 的乘数（64 位的 FNV prime，无单位）。
const PRIME: u64 = 0x0100_0000_01b3;

/// 逐段喂入字节的 FNV-1a 哈希器。
///
/// 分几次 [`Fnv::write`] 与把各段拼在一起一次写入的结果相同，因此无需先把各段拼成一个缓冲区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv::new()
    }
}

impl Fnv {
    /// 创建一个还没有喂入任何字节的哈希器。
    pub const fn new() -> Self {
        Fnv(OFFSET)
    }

    /// 依次喂入 `bytes` 中的全部字节。
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.push(b);
        }
    }

    /// 喂入一个字节。
    pub fn push(&mut self, byte: u8) {
        self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(PRIME);
    }

    /// 返回到目前为止喂入的全部字节的哈希值。
    pub const fn finish(self) -> u64 {
        self.0
    }
}

/// 返回 `bytes` 中全部字节的 FNV-1a 哈希值。
pub fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hasher = Fnv::new();
    for b in bytes {
        hasher.push(b);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests;
