//! 需要整形的文字在编辑区中的绘制：组合符号叠在所属的字上，不画进后一个字的显示列。

use super::*;

/// 打开内容为 `text` 的文件并绘制一帧，返回编辑区第 0 行的像素和该区域的宽度（物理像素）。
fn first_row(name: &str, text: &str) -> (Vec<u32>, usize) {
    let dir = sample_folder(name);
    let path = dir.join("sample.txt");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let (area, lh) = (app.layout.text, app.fonts.line_height);
    let renderer = app.frame.renderer.as_ref().unwrap();
    let scale = renderer.scale();
    let image = renderer.snapshot().unwrap();
    let px = |v: f32| (v * scale) as usize;
    let (x0, x1, width) = (px(area.x), px(area.right()), image.width as usize);
    let pixels = (px(area.y)..px(area.y + lh))
        .flat_map(|y| image.pixels[y * width + x0..y * width + x1].to_vec())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (pixels, x1 - x0)
}

/// 返回两幅画面中不同的像素所在的横向范围（物理像素，左闭右开）；完全相同时为 `None`。
fn changed_columns(a: &[u32], b: &[u32], width: usize) -> Option<(usize, usize)> {
    let xs = a
        .iter()
        .zip(b)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i % width);
    xs.fold(None, |range, x| match range {
        None => Some((x, x + 1)),
        Some((from, to)) => Some((from.min(x), to.max(x + 1))),
    })
}

#[test]
fn combining_marks_sit_on_their_base_letter() {
    let (plain, width) = first_row("editor-mark-plain", "ex\n");
    let (marked, _) = first_row("editor-mark-marked", "e\u{301}x\n");
    let (alone, _) = first_row("editor-mark-alone", "e\n");
    // 去掉 `x` 改动的范围即 `x` 所在的列；组合符号带来的改动应全部在 `x` 的左侧。
    let x = changed_columns(&plain, &alone, width).expect("x 有墨迹");
    let mark = changed_columns(&plain, &marked, width).expect("组合符号有墨迹");
    assert!(mark.1 <= x.0, "组合符号画在 {mark:?}，x 在 {x:?}");
}
