//! `markdown` 的测试辅助：读取排版结果中图形超出内容的宽度和表格单元格的位置，供预览的
//! 界面测试检查和点击。

use super::*;

pub(crate) use winc_preview::markdown::slug;

/// 绘制的框和线超出内容宽度的量（逻辑像素，未超出时不大于 0），供测试检查图是否完整显示。
pub(crate) fn overflow(app: &App) -> f32 {
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return 0.0;
    };
    let right = laid
        .blocks
        .iter()
        .flat_map(|block| block.block.items.iter())
        .filter_map(|item| match item {
            Item::Fill { rect, .. } | Item::Path { rect, .. } | Item::Polygon { rect, .. } => {
                Some(rect.right())
            }
            _ => None,
        })
        .fold(0.0, f32::max);
    right - laid.key.width
}

/// 第一张表格各单元格在窗口中的范围，按行排列，供测试点击单元格。
pub(crate) fn table_cells(app: &App) -> Vec<Vec<Rect>> {
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return Vec::new();
    };
    let origin = app.preview.markdown.area.y + laid.pad - app.preview.markdown.scroll();
    for (block, top) in laid.blocks.iter().zip(&laid.tops) {
        for item in block.block.items.iter() {
            if let Item::Table {
                rect, cols, rows, ..
            } = item
            {
                let mut y = origin + top + rect.y;
                return rows
                    .iter()
                    .map(|h| {
                        let mut x = laid.left + rect.x;
                        let row = cols
                            .iter()
                            .map(|w| {
                                let cell = Rect::new(x, y, *w, *h);
                                x += w;
                                cell
                            })
                            .collect();
                        y += h;
                        row
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}

/// 分栏时编辑区光标所在行的竖条（窗口坐标），与绘制时相同；没有该光标时返回 `None`。
pub(crate) fn mirrored_caret_bar(app: &App) -> Option<Rect> {
    let laid = app.preview.markdown.layout.as_ref()?;
    let (_, y, h) = laid.caret_point(app.markdown_caret_source()?)?;
    let max = (app.preview.markdown.height - app.preview.markdown.area.h).max(0.0);
    let scroll = app.preview.markdown.scroll().min(max);
    Some(caret_bar(
        laid,
        (y, h),
        app.preview.markdown.area.y - scroll,
    ))
}

#[test]
fn a_skipped_dragged_block_still_registers_its_images() {
    use std::collections::HashMap;
    use std::path::PathBuf;

    let path = PathBuf::from("assets/example.png");
    let items = vec![Item::Image {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        path: path.clone(),
        at: None,
    }];
    let slots: HashMap<PathBuf, ImageSlot> = HashMap::new();
    let (mut shown, mut wanted) = (Vec::new(), Vec::new());
    note_images(&items, 2.0, &slots, &mut shown, &mut wanted);
    assert_eq!(shown, std::slice::from_ref(&path));
    assert_eq!(wanted, [(path.clone(), (20, 20))]);
}
