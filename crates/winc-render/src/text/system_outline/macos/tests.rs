//! `macos` 模块的测试辅助：向 CoreText 查询字形的前进宽度，供核对字宽的测试使用。
//!
//! 只在测试构建下编译。

use super::*;

impl SystemOutlines {
    /// 返回 CoreText 给出的字形 `glyph` 的前进宽度，按 1 em = `em` 缩放。
    ///
    /// 测试中用来核对 swash 读出的字宽。
    pub(crate) fn advance(&self, mut glyph: u16, em: f32) -> f32 {
        // SAFETY: `glyph` 是本函数的局部变量，调用期间有效；不需要逐个字形的结果，
        // `advances` 传 null，CoreText 只返回总宽度。
        let total = unsafe {
            self.font.advances_for_glyphs(
                objc2_core_text::CTFontOrientation::Horizontal,
                NonNull::from(&mut glyph),
                std::ptr::null_mut(),
                1,
            )
        };
        total as f32 * em / SIZE as f32
    }
}
