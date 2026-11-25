//! 语义补全的列表：光标下方的卡片，每行是候选的种类图标与名字；下方放不下时显示在光标
//! 上方。样式与 Markdown 的链接补全列表一致（同一个卡片底色、圆角和行高）。

use super::popup_menu::{MARGIN, ROW_INSET, band_ink};
use super::*;
use crate::app::lsp::COMPLETION_VISIBLE;
use crate::app::{POPUP_PAD, ScrollArea};
use crate::icons::{symbol, SymbolGlyph, FILE_WIDTH};
use winc_i18n::t;

/// 卡片的最小、最大宽度（逻辑像素）。
const WIDTH: (f32, f32) = (220.0, 420.0);
/// 每行的高度（逻辑像素）。
const LIST_ROW: f32 = 26.0;
/// 每行底色上下各缩进的距离（逻辑像素）。
const BAND_INSET: f32 = 2.0;
/// 行内左右两侧的留白（逻辑像素）。
const PAD: f32 = 10.0;
/// 行首种类图标与名字之间的间距（逻辑像素）；图标本身宽 [`FILE_WIDTH`]。
const ICON_GAP: f32 = 7.0;
/// 名字右边留给种类文字的总宽度（逻辑像素）：种类与名字之间的间距，加上最长的种类文字。
const KIND_ROOM: f32 = 72.0;
/// 名字与种类之间的间距（逻辑像素）。
const KIND_GAP: f32 = 10.0;

/// 候选种类的短名（按 LSP 的 `CompletionItemKind` 分成几组）；列表首期只显示这一项与名字。
fn kind_label(kind: Option<u32>) -> &'static str {
    match kind.unwrap_or(0) {
        2..=4 => t!("editor-lsp-kind-function"),
        5 | 10 => t!("editor-lsp-kind-field"),
        6 | 11 | 12 | 21 => t!("editor-lsp-kind-variable"),
        7 | 8 | 13 | 22 | 25 => t!("editor-lsp-kind-type"),
        9 | 17 | 19 => t!("editor-lsp-kind-module"),
        _ => t!("editor-lsp-kind-other"),
    }
}

/// 候选种类的图标与颜色：与大纲的 `symbol_look` 同一套——颜色取语法高亮里同类名字的
/// 颜色（函数用函数色、字段与变量用属性色……），扫一眼行首就能分清函数与字段。
fn kind_look(p: &Palette, kind: Option<u32>) -> (SymbolGlyph, Color) {
    match kind.unwrap_or(0) {
        2..=4 => (SymbolGlyph::Cube, p.syntax_function),
        5 | 6 | 10 | 11 | 12 | 21 => (SymbolGlyph::Field, p.syntax_property),
        7 | 8 | 13 | 22 | 25 => (SymbolGlyph::Card, p.syntax_builtin),
        9 | 17 | 19 => (SymbolGlyph::Braces, p.syntax_keyword),
        _ => (SymbolGlyph::Field, p.text_muted),
    }
}

/// 绘制补全列表（开着、有候选时），记下卡片和各行的位置。
pub(super) fn draw_completion(app: &mut App, frame: &mut Frame<'_>) {
    if !app.completion.shown() || app.completion.rows().is_empty() {
        return;
    }
    let Some(caret) = app.ime.cursor else {
        return;
    };
    let rows: Vec<String> = (app.completion.rows().iter())
        .filter_map(|&i| app.completion.items().get(i))
        .map(|item| item.label.clone())
        .collect();
    let size = app.fonts.ui;
    let widest = rows.iter().map(|label| frame.measure(FontKind::Ui, size, label)).fold(0.0, f32::max);
    let area = app.layout.editor;
    let w = (PAD * 2.0 + FILE_WIDTH + ICON_GAP + widest + KIND_ROOM)
        .clamp(WIDTH.0, WIDTH.1)
        .min(area.w - 2.0 * MARGIN);
    let shown = rows.len().min(COMPLETION_VISIBLE);
    let scrolls = rows.len() > shown;
    let h = 2.0 * POPUP_PAD + shown as f32 * LIST_ROW;
    let x = (caret.x)
        .min(area.right() - MARGIN - w)
        .max(area.x + MARGIN);
    let below = caret.bottom() + 4.0;
    let y = if below + h <= area.bottom() - MARGIN {
        below
    } else {
        (caret.y - 4.0 - h).max(area.y + MARGIN)
    };
    let card = Rect::new(x.round(), y.round(), w.round(), h);
    let p = app.palette;
    draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);
    let clip = card.inset(0.0, POPUP_PAD);
    let completion = &mut app.completion;
    let first = completion.first().min(rows.len() - shown);
    let selected = completion.selected();
    completion.scroll_to(first, shown);
    let mut hits = Vec::with_capacity(shown);
    let kinds: Vec<&'static str> = (app.completion.rows().iter())
        .filter_map(|&i| app.completion.items().get(i))
        .map(|item| kind_label(item.kind))
        .collect();
    let looks: Vec<(SymbolGlyph, Color)> = (app.completion.rows().iter())
        .filter_map(|&i| app.completion.items().get(i))
        .map(|item| kind_look(p, item.kind))
        .collect();
    for (row_index, label) in rows.iter().enumerate().skip(first).take(shown) {
        let top = card.y + POPUP_PAD + (row_index - first) as f32 * LIST_ROW;
        let row = Rect::new(card.x, top, card.w, LIST_ROW);
        let band = row.inset(ROW_INSET, BAND_INSET);
        let (ink, kind_ink) = if row_index == selected {
            frame.rect_clipped(band, p.accent_fill, RADIUS_ROW, clip);
            band_ink(p)
        } else {
            if app.pointer.hover == Hover::Completion(row_index) {
                frame.rect_clipped(band, p.hover, RADIUS_ROW, clip);
            }
            (p.text, p.text_muted)
        };
        // 行首画种类图标（与文件树、大纲同风格的线条图标，高 15 逻辑像素），名字让出一段。
        let (glyph, icon_color) = looks
            .get(row_index)
            .copied()
            .unwrap_or((SymbolGlyph::Field, p.text_muted));
        symbol(
            frame,
            (row.x + PAD, row.y + (row.h - 15.0) / 2.0),
            glyph,
            icon_color,
            clip,
        );
        // 候选项的文字可能与已输入的前缀重复，这里只画候选本身（服务器给的已经是完整名字）。
        let text_x = row.x + PAD + FILE_WIDTH + ICON_GAP;
        let kind = kinds.get(row_index).copied().unwrap_or_default();
        let kind_w = frame.measure(FontKind::Ui, app.fonts.ui_small, kind);
        let room = band.right() - PAD - kind_w - KIND_GAP - text_x;
        let shown_label = ellipsize(frame, label, size, room.max(0.0));
        ui_text(frame, app, text_x, row, &shown_label, ink, clip);
        ui_text_small(frame, app, band.right() - PAD - kind_w, row, kind, kind_ink, clip);
        hits.push((row, row_index));
    }
    // 项数多于可见行数时画滚动条，样式与其他列表相同。
    if scrolls {
        let track = Rect::new(card.x, card.y + POPUP_PAD, card.w, shown as f32 * LIST_ROW);
        let (scroll, content) = (first as f32 * LIST_ROW, rows.len() as f32 * LIST_ROW);
        let owner = app.editor.active().map_or(0, |doc| doc.id.0);
        draw_scrollbar(app, frame, ScrollArea::Suggest, owner, track, scroll, content);
    }
    app.hits.completion = hits;
    app.hits.completion_card = Some(card);
}
