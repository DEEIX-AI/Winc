//! 图片预览缓存的尺寸边界与测试辅助。

use super::*;

#[test]
fn raster_cache_shrinks_but_never_exceeds_retained_pixels() {
    let source = Source::Raster(winc_preview::image::Pixels {
        width: 2560,
        height: 1280,
        rgba: Vec::new(),
    });
    assert_eq!(cache_size(&source, (640, 320)), (640, 320));
    assert_eq!(cache_size(&source, (2560, 1280)), (2560, 1280));
    assert_eq!(cache_size(&source, (131_072, 65_536)), (2560, 1280));
    // 极细图片的目标尺寸可能因物理像素取整而只有一边超过原图。
    assert_eq!(cache_size(&source, (2561, 1279)), (2560, 1279));
}

#[test]
fn vector_cache_bounds_the_longest_edge_and_preserves_aspect() {
    let picture = winc_preview::svg::load(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"/>"#,
        None,
        None,
    )
    .unwrap();
    assert_eq!(cache_size(&picture.source, (512, 512)), (512, 512));
    assert_eq!(cache_size(&picture.source, (32_768, 16_384)), (4096, 2048));
    assert_eq!(cache_size(&picture.source, (16_384, 32_768)), (2048, 4096));
    assert_eq!(cache_size(&picture.source, (131_072, 1)), (4096, 1));
}

#[test]
fn checker_bounds_keep_phase_and_limit_work_to_visible_cells() {
    let rect = Rect::new(-1002.0, -998.0, 200_000.0, 200_000.0);
    let clip = Rect::new(10.0, 20.0, 80.0, 70.0);
    let bounds = checker_bounds(rect, clip, 8.0);
    assert_eq!(bounds, Rect::new(6.0, 10.0, 84.0, 80.0));
    assert_eq!((bounds.x - rect.x) % 16.0, 0.0);
    assert_eq!((bounds.y - rect.y) % 16.0, 0.0);
    assert!(bounds.w < clip.w + 16.0 && bounds.h < clip.h + 16.0);
    assert_eq!(bounds.intersect(clip), rect.intersect(clip));

    let small = Rect::new(12.0, 24.0, 11.0, 13.0);
    assert_eq!(checker_bounds(small, clip, 8.0), small);
    assert_eq!(
        checker_bounds(Rect::new(-40.0, -40.0, 10.0, 10.0), clip, 8.0),
        Rect::default()
    );
}

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
