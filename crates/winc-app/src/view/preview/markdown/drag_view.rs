//! Markdown 预览中的拖动浮层：浮动卡片、抬起阴影、把手和列表项裁剪。

use super::*;
use crate::app::preview::markdown::drag;

/// 画鼠标所指单元的拖动把手，以及拖动中的插入位置指示线和浮层。
///
/// 把手画在正文左侧的留白中，与单元的第一行对齐（见 [`drag::shows_handles`] 和 [`drag::gutter`]）。
/// 拖动中的浮层跟着鼠标；被拖的块和列表项在绘制时都跳过自己的原文，给让位的兄弟项
/// 腾出位置。插入位置指示线在最近的边界处，换位置时滑过去；不能真正改变顺序时不画。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let p = app.palette;
    let (mx, my) = app.mouse();
    let md = &app.preview.markdown;
    let Some(laid) = md.layout.as_ref() else {
        return;
    };
    if md.drag.session.is_none()
        && let Some(element) = md.element_at(app, mx, my)
        && let Some(rect) = md.handle_rect(app, element)
    {
        draw_grip(frame, p, rect, rect.contains(mx, my), area);
    }
    let Some(drag) = md.drag.session.as_ref() else {
        if let Some(card) = md.drag.drop_card.as_ref() {
            draw_drag_card(
                frame,
                p,
                area,
                md,
                laid,
                CardView {
                    block: card.block,
                    left: card.left,
                    width: card.width,
                    height: card.height,
                    offset: card.offset,
                    top: card.top,
                    item: matches!(card.element, drag::Element::Item { .. }),
                },
            );
        }
        return;
    };
    let origin = area.y + laid.pad - drawn_scroll(md.scroll());
    let lifted = drag.lifted();
    // 插入位置指示线：横贯正文一栏的强调色细线，左端一个小圆点。抬起时淡入；还没离开原位
    // （没有可以插入的其他位置）时不画。先画指示线，浮层盖住中间一段，两侧露出的线仍指出落点。
    if drag.indicator.is_some() {
        let y = (origin + drag.boundary()).round();
        frame.with_opacity(lifted, |frame| {
            frame.rect_clipped(
                Rect::new(
                    laid.left,
                    y - drag::INDICATOR / 2.0,
                    laid.width(),
                    drag::INDICATOR,
                ),
                drag::INDICATOR / 2.0,
                p.accent,
                area,
            );
            frame.rect_clipped(
                Rect::new(laid.left - 5.0, y - 2.5, 5.0, 5.0),
                2.5,
                p.accent,
                area,
            );
        });
    }
    // 浮层：被拖动的单元连同阴影一起浮在鼠标处，原槽由让位的兄弟项占据。
    let top = (my - drag.grab_y - 3.0 * lifted).clamp(
        area.y + 6.0,
        (area.bottom() - drag.height - 6.0).max(area.y + 6.0),
    );
    let view = CardView {
        block: drag.block,
        left: drag.left,
        width: drag.width,
        height: drag.height,
        offset: drag.offset,
        top,
        item: matches!(drag.element, drag::Element::Item { .. }),
    };
    frame.with_opacity(0.55 + 0.45 * lifted, |frame| {
        draw_drag_card(frame, p, area, md, laid, view);
    });
    // 拖动按钮留在左侧留白里、跟着按下时的高度：松手前一直看得见它，也不压住行号。
    let grip = Rect::new(
        drag::handle_left(drag.left, md.drag.numbers).max(area.x),
        top + drag.grab_y.min(drag.height) - 10.0,
        drag::HANDLE,
        20.0,
    );
    frame.with_opacity(0.55 + 0.45 * lifted, |frame| {
        draw_grip(frame, p, grip, false, area);
    });
}

/// 浮动卡片的位置和内容范围。
#[derive(Clone, Copy)]
struct CardView {
    block: usize,
    left: f32,
    width: f32,
    height: f32,
    offset: f32,
    top: f32,
    item: bool,
}

/// 画拖动中的浮动卡片：阴影、边框、底色、左侧强调条和内容。
fn draw_drag_card(
    frame: &mut Frame<'_>,
    p: &Palette,
    area: Rect,
    md: &crate::app::preview::markdown::MarkdownPreview,
    laid: &Laid,
    view: CardView,
) {
    let card = Rect::new(
        view.left - 8.0,
        view.top - 6.0,
        view.width + 16.0,
        view.height + 12.0,
    );
    frame.shadow(card, 8.0, 14.0, 4.0, p.shadow.alpha(0.22), None);
    frame.rect_clipped(card.inset(-1.0, -1.0), 9.0, p.border.alpha(0.8), area);
    frame.rect_clipped(card, 8.0, p.editor_bg, area);
    frame.rect_clipped(
        Rect::new(card.x, card.y + 6.0, 2.0, (card.h - 12.0).max(0.0)),
        1.0,
        p.accent.alpha(0.55),
        area,
    );
    if let Some(block) = laid.blocks.get(view.block) {
        // 列表项浮层只显示这一项：块的各项按项在块中的纵向偏移平移，再裁剪到这一项的内容
        // 范围；上下各收 2 像素，下一项的字形顶部不会露进卡片。
        let inset = if view.item { 2.0 } else { 0.0 };
        let content = Rect::new(
            view.left,
            view.top + inset,
            view.width,
            (view.height - inset * 2.0).max(1.0),
        );
        draw_items(
            frame,
            &block.block.items,
            (view.left, view.top - view.offset),
            content,
            Some(p),
            Some(&md.images),
            frame.scale(),
        );
    }
}

/// 在 `rect` 中画拖动把手：两列三行的圆点；鼠标在把手上时铺一层悬停底色。
fn draw_grip(frame: &mut Frame<'_>, p: &Palette, rect: Rect, over: bool, clip: Rect) {
    if over {
        // 底色比方块小一圈，不与左边的折叠箭头连在一起。
        frame.rect_clipped(rect.inset(1.0, 1.0), 4.0, p.hover, clip);
    }
    let color = if over { p.text } else { p.text_muted };
    let dot = 1.8;
    let gap = 3.0;
    let x = rect.x + (rect.w - (2.0 * dot + gap)) / 2.0;
    let y = rect.y + (rect.h - (3.0 * dot + 2.0 * gap)) / 2.0;
    for row in 0..3 {
        for col in 0..2 {
            frame.rect_clipped(
                Rect::new(
                    x + col as f32 * (dot + gap),
                    y + row as f32 * (dot + gap),
                    dot,
                    dot,
                ),
                dot / 2.0,
                color,
                clip,
            );
        }
    }
}

/// 这一项是否属于正在拖动的列表项：有原文位置时按原文范围判断，列表圆点、勾选框等
/// 没有原文位置的项按它所在的纵向范围判断。
pub(super) fn in_dragged_item(
    item: &Item,
    range: std::ops::Range<usize>,
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

/// 拖动中的列表项在每个块中的原文字节范围和内容坐标上下边缘。
pub(super) fn dragged_items(app: &App) -> Vec<Option<(std::ops::Range<usize>, f32, f32)>> {
    let blocks = app
        .preview
        .markdown
        .layout
        .as_ref()
        .map_or(0, |laid| laid.blocks.len());
    (0..blocks)
        .map(|block| app.preview.markdown.dragged_item(app, block))
        .collect()
}
