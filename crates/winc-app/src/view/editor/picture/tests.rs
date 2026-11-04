//! 图片预览缓存的测试辅助：暴露缩放后的物理像素尺寸。

use super::*;

impl PictureCache {
    /// 返回缩放后的宽度（物理像素）。
    pub(crate) fn width(&self) -> u32 {
        self.pixels.width
    }

    /// 返回缩放后的高度（物理像素）。
    pub(crate) fn height(&self) -> u32 {
        self.pixels.height
    }
}
