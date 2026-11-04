//! `glyphs` 的测试辅助：读取字形缓存中的字形个数，供检查缓存复用的测试使用。
//!
//! 只在测试构建下编译。

use super::*;

impl GlyphCache {
    /// 返回缓存中的字形个数。
    pub(crate) fn cached(&self) -> usize {
        self.glyphs.len()
    }
}
