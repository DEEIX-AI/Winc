//! Markdown 预览中的拖动绘制：把手、插入位置指示线、浮动卡片，以及拖动对正文绘制的影响。

use std::ops::Range;

use super::*;
use crate::app::preview::markdown::MarkdownPreview;
use crate::app::preview::markdown::drag::{self, Element, ItemSlide};

/// 浮动卡片在内容左右两侧的留白（逻辑像素，取自调参）：略大于纵向留白 [`drag::CARD_PAD_Y`]，
/// 卡片边缘不贴着文字。
const CARD_PAD_X: f32 = 8.0;
/// 浮动卡片的圆角半径（逻辑像素），等于左右留白，圆角不切入内容。
const CARD_RADIUS: f32 = CARD_PAD_X;
/// 刚按下时浮层和把手的不透明度（取自调参）：抬起过程中升到 1，原位置的内容不被突然盖住。
const PRESSED_OPACITY: f32 = 0.55;
/// 把手圆点的直径和圆点之间的间距（逻辑像素，取自调参）：两列三行的圆点放进 [`drag::HANDLE`]
/// 宽、[`drag::HANDLE_HEIGHT`] 高的方块后四周仍有留白。
const DOT: f32 = 1.8;
const DOT_GAP: f32 = 3.0;
/// 列表项浮层内容在上下各收进的距离（逻辑像素，取自调参）：相邻项的字形顶部不露进卡片。
const ITEM_INSET: f32 = 2.0;
/// 插入位置指示线左端圆点的直径（逻辑像素），约为指示线粗细的 2.5 倍，从线上能分辨出来。
const INDICATOR_DOT: f32 = 5.0;

/// 一帧中拖动对正文绘制的影响：原位置不绘制的块、让位偏移和被拖动的列表项。
pub(super) struct DragPaint {
    /// 由浮动卡片代替、原位置不绘制的块。
    hidden: Option<usize>,
    /// 各顶层块是否按拖动中或松手时的让位偏移移动。
    blocks_shifted: bool,
    /// 列表项的偏移：（列表所在的块，各项的偏移）。
    slide: Option<(usize, ItemSlide)>,
    /// 被拖动的列表项：（所在的块，原文字节范围（相对顶层块），内容坐标中的上下边缘）。
    item: Option<(usize, Range<usize>, f32, f32)>,
}

impl DragPaint {
    /// 第 `block` 块是否由浮动卡片代替，原位置不绘制。
    pub(super) fn hides(&self, block: usize) -> bool {
        self.hidden == Some(block)
    }

    /// 返回第 `block` 块这一帧的让位偏移（逻辑像素）。
    pub(super) fn block_shift(&self, md: &MarkdownPreview, block: usize) -> f32 {
        if !self.blocks_shifted {
            return 0.0;
        }
        match (&md.drag.session, &md.drag.drop_card) {
            (Some(drag), _) => drag.shift.at(block),
            (None, Some(card)) => card.shift.get(block).copied().unwrap_or(0.0),
            (None, None) => 0.0,
        }
    }

    /// 返回第 `block` 块（上边缘在内容坐标 `top`）中 `item` 这一帧的纵向偏移（逻辑像素）；
    /// `item` 属于被拖动的列表项、由浮动卡片代替时为 `None`。
    pub(super) fn item_offset(
        &self,
        md: &MarkdownPreview,
        block: usize,
        top: f32,
        item: &Item,
    ) -> Option<f32> {
        if let Some((dragged, range, y0, y1)) = &self.item
            && *dragged == block
            && in_dragged_item(item, range, top, *y0, *y1)
        {
            return None;
        }
        match &self.slide {
            Some((slid, slide)) if *slid == block => {
                let (y0, y1) = item.span_y();
                Some(slide.offset(md, top + (y0 + y1) / 2.0))
            }
            _ => Some(0.0),
        }
    }
}

/// 绘制正文之前推进拖动的状态，返回这一帧拖动对正文绘制的影响；没有进行中的拖动、等待中的
/// 浮动卡片和列表项滑动时立即返回 `None`，不做其他计算。
pub(super) fn prepare(app: &mut App) -> Option<DragPaint> {
    let md = &app.preview.markdown;
    if md.drag.session.is_none() && md.drag.drop_card.is_none() && md.drag.item_motion.is_none() {
        return None;
    }
    app.keep_markdown_drag_current();
    // 新排版到达或文档再次修改后，移除松手时保留的浮动卡片。
    if let Some(revision) = app.editor.active().map(|doc| doc.revision) {
        app.preview.markdown.clear_drop_card(revision);
    }
    app.update_markdown_drag_shift();
    // 新的解析和排版到达后才启动列表项滑动，不按旧排版计算新顺序。
    app.start_item_motion();
    let md = &mut app.preview.markdown;
    if md
        .drag
        .item_motion
        .as_ref()
        .is_some_and(drag::ItemMotion::finished)
    {
        md.drag.item_motion = None;
    }
    let md = &app.preview.markdown;
    let motion = md.drag.item_motion.as_ref().map(|motion| motion.block);
    let sliding = motion.and_then(|block| Some((block, md.item_slide(app, block)?)));
    let dragged = md.dragged_element();
    Some(DragPaint {
        hidden: dragged.and_then(|element| match element {
            Element::Block(block) => Some(block),
            Element::Item { .. } => None,
        }),
        blocks_shifted: matches!(dragged, Some(Element::Block(_))),
        slide: sliding.or_else(|| md.drag_item_slide(app)),
        item: md.dragged_item(app),
    })
}

/// 为把手和折叠箭头在正文左侧预留 [`drag::gutter`] 的宽度：正文右移、变窄，右边缘不超过
/// `right`。返回新的左边缘和宽度（逻辑像素）。
pub(super) fn make_room(numbers: f32, (left, width): (f32, f32), right: f32) -> (f32, f32) {
    let gutter = drag::gutter(numbers);
    let overflow = (left + width + gutter - right).max(0.0);
    (left + gutter, (width - overflow).max(1.0).floor())
}

/// 绘制指针所指单元的拖动把手，以及拖动中的插入位置指示线和浮动卡片。
///
/// 把手画在正文左侧的留白中，与单元的第一行对齐（见 [`drag::gutter`]）。插入位置指示线在
/// 最近的边界处，换位置时滑过去；放置位置不改变顺序时不画。
pub(super) fn draw(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    let (mx, my) = app.mouse();
    let md = &app.preview.markdown;
    let Some(laid) = md.layout.as_ref() else {
        return;
    };
    if md.drag.session.is_none()
        && app.cursor_inside()
        && let Some(element) = md.element_at(app, mx, my)
        && let Some(rect) = md.handle_rect(app, element)
    {
        draw_grip(frame, p, rect, rect.contains(mx, my), area);
    }
    let Some(drag) = md.drag.session.as_ref() else {
        if let Some(card) = md.drag.drop_card.as_ref() {
            let view = CardView {
                block: card.block,
                left: card.left,
                width: card.width,
                height: card.height,
                offset: card.offset,
                top: card.top,
                item: matches!(card.element, Element::Item { .. }),
            };
            draw_drag_card(frame, p, area, md, laid, view);
        }
        return;
    };
    let origin = area.y + laid.pad - drawn_scroll(md.scroll());
    let lifted = drag.lifted();
    // 先画指示线，浮层盖住中间一段，两侧露出的线仍指出落点；抬起时淡入。
    if drag.indicator.is_some() {
        let y = (origin + drag.boundary()).round();
        let (line, dot) = (drag::INDICATOR, INDICATOR_DOT);
        frame.with_opacity(lifted, |frame| {
            let bar = Rect::new(laid.left, y - line / 2.0, laid.width(), line);
            frame.rect_clipped(bar, line / 2.0, p.accent, area);
            let end = Rect::new(laid.left - dot, y - dot / 2.0, dot, dot);
            frame.rect_clipped(end, dot / 2.0, p.accent, area);
        });
    }
    let top = drag.card_top(area, my);
    let view = CardView {
        block: drag.block,
        left: drag.left,
        width: drag.width,
        height: drag.height,
        offset: drag.offset,
        top,
        item: matches!(drag.element, Element::Item { .. }),
    };
    // 把手留在左侧留白中，保持按下时相对卡片的高度，不压住行号。
    let grip = Rect::new(
        drag::handle_left(drag.left, app.hits.markdown_numbers).max(area.x),
        top + drag.grab_y.min(drag.height) - drag::HANDLE_HEIGHT / 2.0,
        drag::HANDLE,
        drag::HANDLE_HEIGHT,
    );
    frame.with_opacity(
        PRESSED_OPACITY + (1.0 - PRESSED_OPACITY) * lifted,
        |frame| {
            draw_drag_card(frame, p, area, md, laid, view);
            draw_grip(frame, p, grip, false, area);
        },
    );
}

/// 浮动卡片的位置和内容范围（窗口坐标，逻辑像素）。
#[derive(Clone, Copy)]
struct CardView {
    /// 单元所在的顶层块序号。
    block: usize,
    /// 内容的左边缘、宽度和高度。
    left: f32,
    width: f32,
    height: f32,
    /// 单元上边缘相对顶层块上边缘的偏移。
    offset: f32,
    /// 内容的上边缘。
    top: f32,
    /// 单元是否是列表项。
    item: bool,
}

/// 绘制浮动卡片：阴影、边框、底色、左侧强调条和内容。
fn draw_drag_card(
    frame: &mut Frame<'_>,
    p: &Palette,
    area: Rect,
    md: &MarkdownPreview,
    laid: &Laid,
    view: CardView,
) {
    let (pad_x, pad_y) = (CARD_PAD_X, drag::CARD_PAD_Y);
    let card = Rect::new(
        view.left - pad_x,
        view.top - pad_y,
        view.width + pad_x * 2.0,
        view.height + pad_y * 2.0,
    );
    frame.shadow(card, CARD_RADIUS, 14.0, 4.0, p.shadow.alpha(0.22), None);
    let border = card.inset(-1.0, -1.0);
    frame.rect_clipped(border, CARD_RADIUS + 1.0, p.border.alpha(0.8), area);
    frame.rect_clipped(card, CARD_RADIUS, p.editor_bg, area);
    let accent = Rect::new(card.x, card.y + pad_y, 2.0, (card.h - pad_y * 2.0).max(0.0));
    frame.rect_clipped(accent, 1.0, p.accent.alpha(0.55), area);
    let Some(entry) = laid.blocks.get(view.block) else {
        return;
    };
    // 列表项浮层只显示该项：块中各项按该项在块中的纵向偏移平移，再裁剪到该项的内容范围。
    let inset = if view.item { ITEM_INSET } else { 0.0 };
    let content = Rect::new(
        view.left,
        view.top + inset,
        view.width,
        (view.height - inset * 2.0).max(1.0),
    );
    let origin = (view.left, view.top - view.offset);
    draw_card_items(frame, p, &md.images, &entry.block.items, origin, content);
}

/// 以 `(left, top)` 为块的左上角绘制浮动卡片中的 `items`，裁剪到 `clip`。
///
/// 只画外观：链接、表格、代码块这类只登记点击范围的项不画，也不登记点击范围。
fn draw_card_items(
    frame: &mut Frame<'_>,
    p: &Palette,
    images: &std::collections::HashMap<std::path::PathBuf, ImageSlot>,
    items: &[Item],
    (left, top): (f32, f32),
    clip: Rect,
) {
    let at = |r: &Rect| Rect::new(left + r.x, top + r.y, r.w, r.h);
    for item in items {
        let (y0, y1) = item.span_y();
        if top + y1 < clip.y || top + y0 > clip.bottom() {
            continue;
        }
        match item {
            Item::Text {
                x,
                baseline,
                text,
                font,
                color,
                ..
            } => {
                frame.text(*font, (left + x, top + baseline), text, *color, clip);
            }
            Item::Fill {
                rect,
                color,
                radius,
            } => frame.rect_clipped(at(rect), *radius, *color, clip),
            Item::Corners { rect, color, radii } => {
                frame.rect_corners(at(rect), *radii, *color, clip);
            }
            Item::Path {
                strokes,
                width,
                color,
                ..
            } => {
                let moved: Vec<Stroke> = strokes
                    .iter()
                    .map(|&s| winc_typeset::shift(s, left, top))
                    .collect();
                frame.stroke(&moved, *width, *color, clip);
            }
            Item::Polygon { points, color, .. } => {
                let moved: Vec<(f32, f32)> =
                    points.iter().map(|&(x, y)| (x + left, y + top)).collect();
                frame.polygon(&moved, *color, clip);
            }
            Item::Image { rect, path, .. } => {
                let r = at(rect);
                let scale = frame.scale();
                let size = ((r.w * scale).round() as u32, (r.h * scale).round() as u32);
                if let Some(ImageSlot::Ready { pixels, .. }) = images.get(path)
                    && let Some(px) = pixels.iter().find(|px| (px.width, px.height) == size)
                {
                    frame.pixels((r.x, r.y), px.width, px.height, &px.rgba, clip);
                }
            }
            Item::Check { rect, checked, .. } => draw_check(frame, p, at(rect), *checked, clip),
            _ => {}
        }
    }
}

/// 在 `rect` 中绘制拖动把手：两列三行的圆点；指针在把手上时铺一层悬停底色。
fn draw_grip(frame: &mut Frame<'_>, p: &Palette, rect: Rect, over: bool, clip: Rect) {
    if over {
        // 底色比方块小一圈，不与右侧的折叠箭头连在一起。
        frame.rect_clipped(rect.inset(1.0, 1.0), 4.0, p.hover, clip);
    }
    let color = if over { p.text } else { p.text_muted };
    let x = rect.x + (rect.w - (2.0 * DOT + DOT_GAP)) / 2.0;
    let y = rect.y + (rect.h - (3.0 * DOT + 2.0 * DOT_GAP)) / 2.0;
    for row in 0..3 {
        for col in 0..2 {
            let step = DOT + DOT_GAP;
            let dot = Rect::new(x + col as f32 * step, y + row as f32 * step, DOT, DOT);
            frame.rect_clipped(dot, DOT / 2.0, color, clip);
        }
    }
}

/// `item` 是否属于正在拖动的列表项：有原文位置的项按原文范围 `range` 判断；列表记号、勾选框
/// 等没有原文位置的项按中线是否落在该项的上下边缘 `dragged_top..dragged_bottom` 之间判断。
fn in_dragged_item(
    item: &Item,
    range: &Range<usize>,
    block_top: f32,
    dragged_top: f32,
    dragged_bottom: f32,
) -> bool {
    let source = match item {
        Item::Text {
            span: Some(span), ..
        } => span.source.map(|source| source as usize),
        Item::Image {
            at: Some((_, source)),
            ..
        } => Some(*source as usize),
        _ => None,
    };
    if let Some(source) = source {
        return range.contains(&source);
    }
    let (top, bottom) = item.span_y();
    let mid = block_top + (top + bottom) * 0.5;
    mid >= dragged_top && mid < dragged_bottom
}
