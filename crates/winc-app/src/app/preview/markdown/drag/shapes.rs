//! 列表项的纵向几何：每一项的第一行、上下边缘和浮层宽度。

use super::{CENTER_RISE, Laid, ListItems, MIN_CARD};

/// 列表项在预览中的纵向位置和宽度（内容坐标，逻辑像素）。
#[derive(Clone, Copy)]
pub(super) struct ItemShape {
    pub(super) top: f32,
    pub(super) bottom: f32,
    pub(super) center: f32,
    pub(super) width: f32,
}

/// 计算各列表项的第一行位置、范围和浮层宽度。
///
/// 项的起点取该项范围内最靠上的文字；没有文字（只有图片等）时按项数均分块高。下一项的起点
/// 是当前项的终点，最后一项到块底。
pub(super) fn item_shapes(laid: &Laid, block: usize, items: &ListItems) -> Option<Vec<ItemShape>> {
    let block_top = *laid.tops.get(block)?;
    let height = laid.blocks.get(block)?.block.height;
    let width = laid.width();
    let count = items.ranges.len();
    if count == 0 {
        return None;
    }
    let mut shapes = vec![
        ItemShape {
            top: f32::MAX,
            bottom: 0.0,
            center: f32::NAN,
            width: 0.0,
        };
        count
    ];
    for (top, _, x, baseline, size, span, _, _) in laid.spans(block..block + 1) {
        let Some(source) = span.source else {
            continue;
        };
        let source = source as usize;
        let item = items
            .ranges
            .partition_point(|range| range.start <= source)
            .saturating_sub(1);
        let Some(range) = items.ranges.get(item) else {
            continue;
        };
        if source >= range.end {
            continue;
        }
        let line_top = top + baseline - size * 1.05;
        let shape = &mut shapes[item];
        shape.top = shape.top.min(line_top);
        if shape.center.is_nan() {
            shape.center = top + baseline - size * CENTER_RISE;
        }
        shape.width = shape.width.max(x + span.advances.iter().sum::<f32>());
    }
    // 图片没有字宽记录，按图片的外框计入项宽。
    for item in &laid.blocks.get(block)?.block.items {
        if let winc_typeset::Item::Image {
            rect,
            at: Some((_, source)),
            ..
        } = item
        {
            let source = *source as usize;
            let index = items
                .ranges
                .partition_point(|range| range.start <= source)
                .saturating_sub(1);
            if let Some(range) = items.ranges.get(index)
                && source < range.end
            {
                shapes[index].width = shapes[index].width.max(rect.right());
            }
        }
    }
    // 没有文字的项按项数均分块高，至少给出一个可用的位置。
    for (i, shape) in shapes.iter_mut().enumerate() {
        if shape.top == f32::MAX {
            shape.top = block_top + height * i as f32 / count as f32;
        }
    }
    for i in 0..count {
        let top = shapes[i].top;
        let bottom = shapes
            .get(i + 1)
            .map_or(block_top + height, |shape| shape.top);
        let shape = &mut shapes[i];
        shape.bottom = bottom;
        if shape.center.is_nan() {
            shape.center = top + (bottom - top) * 0.5;
        }
        shape.width = shape.width.clamp(MIN_CARD, width);
    }
    Some(shapes)
}
