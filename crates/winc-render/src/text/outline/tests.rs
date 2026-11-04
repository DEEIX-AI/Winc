//! 轮廓字体的测试：通用字族名映射到系统字体，粗体、斜体和后备字体都能取到轮廓。

use super::*;

#[cfg(target_os = "macos")]
#[test]
fn system_fonts_give_outlines_bold_italic_and_fallbacks() {
    let fonts = OutlineFonts::new();
    let sans = ["sans-serif".to_owned()];
    let a = fonts.glyph(&sans, 400, false, 'A').expect("Helvetica 有 A");
    assert!(a.advance > 0.5 && a.advance < 0.8, "{}", a.advance);
    assert!(!a.outline.is_empty());
    // 粗体的前进宽度不小于常规体。
    let bold = fonts.glyph(&sans, 700, false, 'A').unwrap();
    assert!(bold.advance >= a.advance);
    // 字体里没有的中文字符，从后备字体中查找。
    assert!(
        fonts
            .glyph(&["Helvetica".to_owned()], 400, false, '中')
            .is_some()
    );
    // 无法识别的字族名由后备字体兜底。
    assert!(
        fonts
            .glyph(&["绝不存在的字体".to_owned()], 400, false, 'x')
            .is_some()
    );
    let m = fonts.metrics(&sans, 400, false);
    assert!(m.ascent > 0.6 && m.descent > 0.1, "{m:?}");
}

/// 回归：查过的字族名（包括找不到的）只增不减；字族名来自 SVG 文件，列出成千上万个名字时
/// 记下的名字和打开的字体数量会持续增长。
///
/// 超过上限时整体清空，之后仍能取到轮廓；断言逐轮检查两个缓存的大小都不超过上限。
#[test]
fn family_lookups_stay_bounded() {
    let fonts = OutlineFonts::new();
    for i in 0..MAX_FAMILIES * 3 {
        let _ = fonts.glyph(&[format!("winc-no-such-family-{i}")], 400, false, 'a');
        let inner = fonts.inner.lock().unwrap();
        assert!(inner.lookup.len() <= MAX_FAMILIES);
        assert!(inner.faces.len() <= MAX_FAMILIES * 4);
    }
    // 系统的后备字体只在 macOS 上一定有。
    if cfg!(target_os = "macos") {
        assert!(fonts.glyph(&[], 400, false, 'a').is_some());
    }
}
