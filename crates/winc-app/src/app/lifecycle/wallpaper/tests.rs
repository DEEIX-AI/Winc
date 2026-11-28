//! 背景图层的测试：窗口所需的大小、何时重新生成，以及磁盘缓存。

use super::*;
use winc_config::BackgroundFit::{Fill, Fit, Stretch, Tile};

fn source(opacity: u8, fit: BackgroundFit) -> Source {
    let path = PathBuf::new();
    Source { path, opacity, fit }
}

#[test]
fn layers_cover_the_window_without_upscaling() {
    let size = |full, window| target(&source(30, Fill), full, (window, 1.0));
    // 3840 × 2160 的图铺满 1440 × 900 的窗口：按高度缩小。
    assert_eq!(size((3840, 2160), (1440.0, 900.0)), (1600, 900));
    // 长边不超过上限。
    assert_eq!(size((3840, 2160), (3840.0, 2160.0)), (2560, 1440));
    // 图片比窗口小时不放大。
    assert_eq!(size((640, 480), (1440.0, 900.0)), (640, 480));
}

#[test]
fn layers_follow_the_fit_in_physical_pixels() {
    let full = (3840, 2160);
    let window = ((1440.0, 900.0), 1.0);
    let size = |opacity, fit| target(&source(opacity, fit), full, window);
    // 适应：整张可见，按宽度缩小。
    assert_eq!(size(30, Fit), (1440, 810));
    // 拉伸：两个方向各自缩小到窗口的大小。
    assert_eq!(size(30, Stretch), (1440, 900));
    // 平铺：保持原来的大小，长边不超过上限，与不透明度无关。
    assert_eq!(
        (size(30, Tile), size(100, Tile)),
        ((1920, 1080), (1920, 1080))
    );
    assert_eq!(target(&source(30, Tile), (64, 48), window), (64, 48));
    // 缩放系数为 2 的屏幕上按物理像素生成，长边不超过上限。
    let dense = ((1440.0, 900.0), 2.0);
    assert_eq!(target(&source(30, Fill), full, dense), (2560, 1440));
}

/// 回归（#608）：不透明度不超过一半时图层也按物理像素生成。按逻辑像素生成的图层在缩放系数
/// 大于 1 的屏幕上要放大才能铺满窗口，渲染器取最近的像素，斜线和细线呈阶梯状。
#[test]
fn faint_layers_are_made_in_physical_pixels_too() {
    // 缩放 125% 的屏幕上 987.2 × 1090.4 的窗口（物理 1234 × 1363），3200 × 1800 的图铺满。
    let window = ((987.2, 1090.4), 1.25);
    for opacity in [15, 30, 50, 55, 100] {
        let size = target(&source(opacity, Fill), (3200, 1800), window);
        assert_eq!(size, (2423, 1363), "{opacity}");
    }
}

#[test]
fn layers_are_remade_when_too_small_or_far_too_large() {
    let full = (3840, 2160);
    let need = |window| target(&source(30, Fill), full, (window, 1.0));
    // 生成时留出余量：1440 × 900 的窗口得到 2000 × 1125 的图层。
    let made = target(&source(30, Fill), full, roomy(((1440.0, 900.0), 1.0)));
    assert_eq!(made, (2000, 1125));
    // 窗口在余量之内变大或变小：不重新生成，由渲染器缩小显示。
    for window in [
        (1440.0, 900.0),
        (1300.0, 800.0),
        (1750.0, 1100.0),
        (1200.0, 900.0),
    ] {
        assert!(!stale(made, need(window)), "{window:?}");
    }
    // 超出余量，哪怕只多一点：重新生成，图层不放大显示。
    assert!(stale(made, need((1440.0, 1130.0))));
    // 窗口小很多：重新生成，释放多占的内存。
    assert!(stale(made, need((900.0, 560.0))));
    // 图片本身很小：窗口再大也没有更清晰的可生成。
    let small = |window| target(&source(30, Fill), (640, 480), (window, 1.0));
    assert!(!stale((640, 480), small((2400.0, 1500.0))));
    // 已到长边的上限：窗口再大也不重新生成。
    let capped = target(&source(30, Fill), full, roomy(((2400.0, 1350.0), 1.0)));
    assert_eq!(capped, (2560, 1440));
    assert!(!stale(capped, need((3000.0, 1700.0))));
    // 平铺的图层须与所需的大小相同。
    let tiled = source(30, Tile);
    let window = ((1440.0, 900.0), 1.0);
    assert!(usable(&tiled, (640, 480), (640, 480), window));
    assert!(!usable(&tiled, (600, 450), (640, 480), window));
    assert!(usable(&source(30, Fill), (640, 480), (640, 480), window));
    // 拉伸时窗口的宽高比变化很大：重新生成。
    let stretched = |window| target(&source(30, Stretch), full, (window, 1.0));
    let made = target(&source(30, Stretch), full, roomy(((1440.0, 900.0), 1.0)));
    assert!(!stale(made, stretched((1440.0, 900.0))));
    assert!(stale(made, stretched((1440.0, 400.0))));
}

#[test]
fn the_cache_returns_the_layer_until_the_image_changes() {
    let dir = std::env::temp_dir().join(format!("winc-wallpaper-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("图.png");
    std::fs::write(&image, b"x").unwrap();
    let stamp = cache::Stamp::of(&image, &std::fs::metadata(&image).unwrap());
    let file = dir.join("wallpaper.cache");
    assert!(cache::read(&file, &stamp).is_none());

    let rgba = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let layer = cache::Cached {
        size: (2, 1),
        full: (4, 2),
        rgba: rgba.clone(),
    };
    cache::write(&file, &stamp, &layer);
    let back = cache::read(&file, &stamp).unwrap();
    assert_eq!((back.size, back.full, back.rgba), ((2, 1), (4, 2), rgba));

    // 图片被替换（大小不同）：缓存作废。
    std::fs::write(&image, b"xy").unwrap();
    let changed = cache::Stamp::of(&image, &std::fs::metadata(&image).unwrap());
    assert!(cache::read(&file, &changed).is_none());
    // 缓存文件不完整：作废。
    let bytes = std::fs::read(&file).unwrap();
    std::fs::write(&file, &bytes[..bytes.len() - 1]).unwrap();
    assert!(cache::read(&file, &stamp).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
