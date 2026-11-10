//! 背景图层的测试：窗口所需的大小、何时重新生成，以及磁盘缓存。

use super::*;

#[test]
fn layers_cover_the_window_without_upscaling() {
    // 3840 × 2160 的图铺满 1440 × 900 的窗口：按高度缩小。
    assert_eq!(target((3840, 2160), (1440.0, 900.0)), (1600, 900));
    // 长边不超过上限。
    assert_eq!(target((3840, 2160), (3840.0, 2160.0)), (1920, 1080));
    // 图片比窗口小时不放大。
    assert_eq!(target((640, 480), (1440.0, 900.0)), (640, 480));
}

#[test]
fn layers_are_remade_only_when_far_from_what_the_window_needs() {
    let full = (3840, 2160);
    let layer = target(full, (1440.0, 900.0));
    // 窗口大小的小幅变化不重新生成，由渲染器缩放。
    for window in [(1300.0, 800.0), (1600.0, 1000.0), (1000.0, 900.0)] {
        assert!(!stale(layer, full, window), "{window:?}");
    }
    // 窗口小很多：重新生成，释放多占的内存。
    assert!(stale(layer, full, (700.0, 450.0)));
    // 窗口大很多：重新生成更清晰的图层。
    let small = target(full, (800.0, 500.0));
    assert!(stale(small, full, (1440.0, 900.0)));
    // 图片本身很小：窗口再大也没有更清晰的可生成。
    assert!(!stale((640, 480), (640, 480), (2400.0, 1500.0)));
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
