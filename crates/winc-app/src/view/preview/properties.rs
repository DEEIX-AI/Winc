//! Properties 预览：在当前窗格中按物理行绘制解码后的文字。

use winc_core::{Lines, text::slice_cells};
use winc_editor::DocumentId;
use winc_render::{Frame, Rect};

use crate::app::{App, ScrollArea};
use crate::layout::{drawn_scroll_line, hscrollbar_thumb};
use crate::theme::TEXT_PAD;
use crate::view::{draw_scrollbar, draw_thumb};

/// 按编辑区的等宽字体和制表符宽度绘制本屏文字。
pub(in crate::view) fn draw(app: &mut App, frame: &mut Frame<'_>, area: Rect, id: DocumentId) {
    app.prepare_properties();
    let Some((text_rect, scroll_x, width)) = app.properties_hscrollbar() else {
        return;
    };
    let lh = app.fonts.line_height;
    let view = &app.preview.properties;
    let scroll = drawn_scroll_line(view.top_line, lh);
    let metrics = app.metrics.mono;
    let lead = (lh - (metrics.ascent + metrics.descent)) / 2.0 + metrics.ascent;
    let tab_width = app.editor.active_tab_width();
    let last_line = app
        .editor
        .active()
        .map_or(0, |doc| doc.text().line_count().saturating_sub(1));
    let origin_x = text_rect.x + TEXT_PAD - scroll_x;
    for (line, chars) in view.lines() {
        let top = f64::from(text_rect.y) + (line as f64 - scroll) * f64::from(lh);
        if top >= f64::from(text_rect.bottom()) {
            break;
        }
        if let Some(((first, from), (last, to))) = view.selection()
            && (first..=last).contains(&line)
        {
            let start = if line == first {
                from.min(chars.len())
            } else {
                0
            };
            let end = if line == last {
                to.min(chars.len())
            } else {
                chars.len()
            };
            let at =
                |col| origin_x + slice_cells(&chars[..col], 0, tab_width) as f32 * app.metrics.cell;
            let left = at(start);
            let right = at(end)
                + if line < last && line < last_line {
                    app.metrics.cell * 0.6
                } else {
                    0.0
                };
            let color = if app.editor_focused() && app.properties_focused() {
                app.palette.selection
            } else {
                app.palette.selection_inactive
            };
            if right > left {
                frame.rect_clipped(
                    Rect::new(left, top.round() as f32, right - left, lh),
                    color,
                    0.0,
                    text_rect,
                );
            }
        }
        let baseline = (top + f64::from(lead)).round() as f32;
        crate::view::editor::draw_glyphs(
            app,
            frame,
            chars,
            &[],
            origin_x,
            baseline,
            tab_width,
            text_rect,
        );
    }
    if let Some((_, scroll, content)) = app.properties_vscrollbar() {
        draw_scrollbar(
            app,
            frame,
            ScrollArea::Properties,
            id.0,
            area,
            scroll,
            content,
        );
    }
    let thumb = hscrollbar_thumb(text_rect, scroll_x, width);
    let look = app.scrollbar_look(
        ScrollArea::PropertiesHorizontal,
        id.0,
        scroll_x,
        thumb.map(|_| text_rect),
    );
    if let Some(thumb) = thumb {
        draw_thumb(frame, app.palette, thumb, look);
    }
}
