//! 编辑区右上角的大纲指示条与悬浮快速查看卡片。
//!
//! 样式与交互参考 Markdown 预览的标题指示条（`view::preview::markdown::draw_toc`）：
//! 平时在编辑区右上角显示紧凑的横线指示条，当前符号高亮并加长；鼠标停留满
//! `TOC_HOVER_DELAY` 后展开悬浮卡片，列出文档符号并支持滚动、点击跳转与右上角图钉固定。

use std::borrow::Cow;

use winc_render::{Color, FontKind, Frame, Rect};

use super::*;
use crate::app::{EditorOutlineTarget, Hover, OutlineKind, ScrollArea};
use crate::theme::{Palette, RADIUS_POPUP, RADIUS_ROW, SCROLLBAR_HIT, TOC_HOVER_DELAY};
use crate::view::workspace::outline::symbol_look;

/// 指示条与文本区右边缘、浮层上边界的间距（逻辑像素）。
const EDGE_RIGHT: f32 = 12.0;
const EDGE_TOP: f32 = 12.0;
/// 指示条中一条的长度（逻辑像素）；当前所在符号为 [`CURRENT`]。
const DASH: f32 = 10.0;
const CURRENT: f32 = 14.0;
/// 指示条中相邻两条的间距（逻辑像素）；符号过多时缩小。
const STEP: f32 = 8.0;
/// 指示条中一条的粗细（逻辑像素）；当前所在符号再粗 1。
const BAR: f32 = 1.5;
/// 鼠标不在指示条上时，当前符号以外各条的不透明度（0–1）。
const REST: f32 = 0.5;
/// 悬浮卡片中一行的高度（逻辑像素）。
const ROW: f32 = 20.0;
/// 悬浮卡片的内边距（逻辑像素）。
const PAD: f32 = 4.0;
/// 每深一层符号，图标与文字缩进的宽度（逻辑像素）。
const INDENT: f32 = 12.0;
/// 符号图标占用的水平空间（逻辑像素）。
const ICON_ROOM: f32 = 18.0;
/// 卡片右上角图钉按钮的宽度（逻辑像素）。
const PIN: f32 = ROW;
/// 悬浮卡片的最小和最大宽度（逻辑像素）。
const WIDTHS: (f32, f32) = (180.0, 320.0);

/// 指示条中一条横线的颜色与粗细：当前之前的符号用次要色，当前符号用强调色并加粗，
/// 之后的用更淡的颜色。
fn dash_look(p: &Palette, i: usize, current: Option<usize>) -> (Color, f32) {
    match current {
        Some(c) if i == c => (p.accent, BAR + 1.0),
        Some(c) if i < c => (p.text_muted, BAR),
        _ => (p.text_dim, BAR),
    }
}

/// 在编辑区右上角绘制大纲指示条与悬浮卡片；可点击范围写入 `app.hits.editor_outline`。
pub(super) fn draw_editor_outline(app: &mut App, frame: &mut Frame<'_>) {
    let editor = app.layout.editor;
    if app.panes.drawing_background
        || !app.editor_outline_active()
        || app.layout.text.w < 120.0
        || editor.h < 60.0
    {
        app.caches.outline_toc.list = Default::default();
        return;
    }
    app.sync_outline(std::time::Instant::now());
    if app.panels.outline.kind != OutlineKind::Symbols || app.panels.outline.items.is_empty() {
        app.caches.outline_toc.list = Default::default();
        return;
    }

    let p = app.palette;
    let total = app.panels.outline.items.len();
    let all_rows: Vec<usize> = (0..total).collect();
    let current = app.outline_current(&all_rows);

    let right_edge =
        (app.layout.text.right() - EDGE_RIGHT).min(editor.right() - SCROLLBAR_HIT - 4.0);
    let top_edge = app.overlay_top().max(editor.y) + EDGE_TOP;
    let avail_h = (editor.bottom() - 8.0 - top_edge).max(0.0);
    if avail_h < ROW {
        app.caches.outline_toc.list = Default::default();
        return;
    }

    let step = (avail_h / total as f32).clamp(BAR + 2.0, STEP);
    let shown = ((avail_h / step).floor() as usize).min(total);
    let x0 = right_edge - CURRENT;
    let y0 = top_edge;

    let rested = app
        .caches
        .outline_toc
        .rest
        .is_some_and(|since| since.elapsed() >= TOC_HOVER_DELAY);
    let on_bar = app.scrollbar_active(crate::layout::Scrollbar::EditorOutline);
    let hovered = rested && (on_bar || matches!(app.pointer.hover, Hover::EditorOutline(_)));
    let pinned = app.state.editor_outline_pinned;
    let closing = app
        .caches
        .outline_toc
        .close
        .map(|clock| 1.0 - clock.progress());
    let open = hovered || pinned || closing.is_some();

    let lit = match app.caches.outline_toc.glow {
        Some((true, clock)) => clock.progress(),
        Some((false, clock)) => 1.0 - clock.progress(),
        None => f32::from(u8::from(app.caches.outline_toc.rest.is_some())),
    };
    let opacity = REST + (1.0 - REST) * lit;
    for i in (0..shown).filter(|_| !open) {
        let (color, thick) = dash_look(p, i, current);
        let color = if current == Some(i) {
            color
        } else {
            color.alpha(color.a * opacity)
        };
        let len = if current == Some(i) { CURRENT } else { DASH };
        let y = y0 + i as f32 * step + (BAR - thick) / 2.0;
        frame.rect_clipped(
            Rect::new(right_edge - len, y, len, thick),
            color,
            thick / 2.0,
            editor,
        );
    }

    let bars_h = (shown.max(1) - 1) as f32 * step + BAR;
    let bars = Rect::new(x0 - 8.0, y0 - 8.0, CURRENT + 16.0, bars_h + 16.0);
    let mut hits = Vec::new();

    if open {
        let size = app.fonts.ui;
        let items = &app.panels.outline.items;
        let widest = items
            .iter()
            .map(|item| {
                let depth = item.depth.min(4) as f32;
                let marks = "#".repeat(usize::from(item.heading));
                let text = if marks.is_empty() {
                    Cow::Borrowed(item.name.as_str())
                } else {
                    Cow::Owned(format!("{marks} {}", item.name))
                };
                depth * INDENT + frame.measure(FontKind::Ui, size, &text)
            })
            .fold(0.0_f32, f32::max);
        let card_w = (widest + PAD * 2.0 + 8.0 + ICON_ROOM + PIN + 8.0)
            .clamp(WIDTHS.0, WIDTHS.1)
            .min((editor.w - 16.0).max(WIDTHS.0.min(editor.w)));

        let fit = ((editor.bottom() - 8.0 - top_edge - PAD * 2.0) / ROW)
            .floor()
            .max(1.0) as usize;
        let rows = total.min(fit);
        let natural = current
            .unwrap_or(0)
            .saturating_sub(rows / 2)
            .min(total - rows);
        let start = app
            .caches
            .outline_toc
            .list
            .start(natural, current, (rows, total));
        let h = rows as f32 * ROW + PAD * 2.0;
        let card_x = (right_edge + PAD - card_w).max(editor.x + 8.0);
        let card_y = (top_edge - PAD)
            .min(editor.bottom() - 8.0 - h)
            .max(editor.y + 8.0);
        let card = Rect::new(card_x, card_y, card_w, h);
        app.caches.outline_toc.list.card = card;
        app.caches.outline_toc.list.row = ROW;

        let progress = match (pinned, hovered, closing) {
            (true, ..) => 1.0,
            (false, false, Some(left)) => left,
            _ => app
                .caches
                .outline_toc
                .fade
                .map_or(1.0, |clock| clock.progress()),
        };
        let grow = |from: f32, to: f32| from + (to - from) * progress;
        let seed = bars.intersect(card);
        let reveal = Rect::new(
            grow(seed.x, card.x),
            grow(seed.y, card.y),
            grow(seed.w, card.w),
            grow(seed.h, card.h),
        );
        let live = hovered || pinned;

        frame.with_opacity(progress, |frame| {
            crate::view::draw_card(frame, p, reveal, RADIUS_POPUP, p.popup_bg);
            for (n, i) in (start..start + rows).enumerate() {
                let item = &app.panels.outline.items[i];
                let band = Rect::new(
                    card.x + PAD,
                    card.y + PAD + n as f32 * ROW,
                    card.w - PAD * 2.0,
                    ROW,
                );
                let item_hovered =
                    app.pointer.hover == Hover::EditorOutline(EditorOutlineTarget::Item(i));
                if item_hovered {
                    frame.rect_clipped(band, p.hover, RADIUS_ROW, reveal);
                } else if current == Some(i) {
                    frame.rect_clipped(band, p.accent.alpha(p.accent.a * 0.10), RADIUS_ROW, reveal);
                }
                let depth = item.depth.min(4) as f32;
                let ix = band.x + 6.0 + depth * INDENT;
                let cy = band.y + band.h / 2.0;
                let (glyph, color) = symbol_look(p, item.kind);
                icons::symbol(frame, (ix, cy - 7.5), glyph, color, reveal);

                let tx = ix + ICON_ROOM;
                let pin_room = if n == 0 { PIN } else { 0.0 };
                let room = (band.right() - 8.0 - pin_room - tx).max(0.0);
                let marks = "#".repeat(usize::from(item.heading));
                let raw = if marks.is_empty() {
                    Cow::Borrowed(item.name.as_str())
                } else {
                    Cow::Owned(format!("{marks} {}", item.name))
                };
                let title = crate::view::ellipsize(frame, &raw, size, room);
                let faux = winc_render::Faux {
                    bold: current == Some(i),
                    ..Default::default()
                };
                let baseline = crate::view::ui_baseline(app, band, size);
                frame.text_faux(
                    FontKind::Ui,
                    faux,
                    size,
                    tx,
                    baseline,
                    &title,
                    p.text,
                    reveal,
                );
                if live {
                    hits.push((band, EditorOutlineTarget::Item(i)));
                }
            }

            let pin = Rect::new(card.right() - PAD - PIN, card.y + PAD, PIN, ROW);
            let pin_hovered = app.pointer.hover == Hover::EditorOutline(EditorOutlineTarget::Pin);
            if pin_hovered {
                frame.rect_clipped(pin, p.hover, RADIUS_ROW, reveal);
            }
            let color = match (pinned, pin_hovered) {
                (true, _) => p.accent,
                (false, true) => p.text,
                (false, false) => p.text_dim,
            };
            let center = (pin.x + pin.w / 2.0, pin.y + pin.h / 2.0);
            icons::pin(frame, center, color, reveal);
            if live {
                hits.insert(0, (pin, EditorOutlineTarget::Pin));
            }
        });

        let track = Rect::new(card.x, card.y + PAD, card.w, rows as f32 * ROW);
        app.caches.outline_toc.list.track = track;
        if let Some((scroll, content)) = app.caches.outline_toc.list.scrollbar().filter(|_| live) {
            let owner = app.editor.active().map_or(0, |doc| doc.id.0);
            crate::view::draw_scrollbar(
                app,
                frame,
                ScrollArea::EditorOutline,
                owner,
                track,
                scroll,
                content,
            );
        }
        if live {
            hits.push((card, EditorOutlineTarget::Bars));
        }
    } else {
        app.caches.outline_toc.list = Default::default();
    }

    hits.push((bars, EditorOutlineTarget::Bars));
    app.hits.editor_outline = hits;
}
