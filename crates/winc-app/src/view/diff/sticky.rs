//! 展开全部内容后的差异粘滞滚动。并排时每侧独立计算，单栏时以顶部可见行所属的版本为准。

use super::*;
use crate::app::{StickyRow, sticky_limit};
use crate::theme::SCROLLBAR_HIT;

pub(super) fn draw(
    app: &mut App,
    frame: &mut Frame<'_>,
    view: &mut DiffView,
    parts: &DiffParts,
    scroll: Scroll,
    pos: f64,
) -> Vec<(Rect, DiffTarget)> {
    let mut hits = Vec::new();
    let max = sticky_limit((parts.body.h / app.fonts.line_height) as usize);
    if view.collapses() || app.config.sticky_scroll == Some(false) || max == 0 {
        return hits;
    }
    if view.is_unified() {
        if let Some((side, first)) = view.unified_row(pos.floor() as usize) {
            hits.extend(draw_side(
                app,
                frame,
                view,
                parts,
                scroll.x,
                (side, first, pos, max),
            ));
        }
    } else {
        for (side, top) in scroll.rows.into_iter().enumerate() {
            hits.extend(draw_side(
                app,
                frame,
                view,
                parts,
                scroll.x,
                (side, top.floor() as usize, top, max),
            ));
        }
    }
    hits
}

fn draw_side(
    app: &mut App,
    frame: &mut Frame<'_>,
    view: &mut DiffView,
    parts: &DiffParts,
    scroll_x: f32,
    (side, first, top, max): (usize, usize, f64, usize),
) -> Vec<(Rect, DiffTarget)> {
    let Some(SideRow::Line { line, part, .. }) = view.rows[side].get(first) else {
        return Vec::new();
    };
    // 展开后文件行连续。按文件行取样，自动换行不会把同一个块重复压入固定区域。
    let last = view
        .line_at(side, view.rows[side].len() - 1)
        .unwrap_or(*line);
    let probes: Vec<_> = (*line..=(*line + max).min(last)).collect();
    let partial = *part > 0 || top.fract() > 0.0;
    let tab_width = app.editor.tab_width;
    let source = if side == LEFT {
        &mut view.old
    } else {
        &mut view.new
    };
    let (scopes, settled) =
        source.sticky_scopes(&probes, partial, max, tab_width, HIGHLIGHT_BUDGET);
    app.highlight_pending |= !settled;

    let lh = app.fonts.line_height;
    let rows: Vec<_> = scopes
        .iter()
        .enumerate()
        .filter_map(|(depth, scope)| {
            let natural = depth as f32 * lh;
            let y = scope.last_row.map_or(natural, |last| {
                let row = view.row_of(side, probes[last] + 1).saturating_sub(1) as f64;
                let row = if view.is_unified() {
                    view.pos_of(side, row)
                } else {
                    row
                };
                natural.min((row - top) as f32 * lh)
            });
            (y + lh > 0.0).then_some(StickyRow {
                line: scope.start,
                depth,
                y,
            })
        })
        .collect();
    let Some(bottom) = rows.iter().map(|row| row.y + lh).reduce(f32::max) else {
        return Vec::new();
    };

    let pane = parts.panes[side];
    let area = pane.rect;
    let p = app.palette;
    let source = view.side(side);
    let origin_x = pane.text.x + DIFF_TEXT_PAD - scroll_x;
    let mut chars = Vec::new();
    let mut tokens = Vec::new();
    frame.within(area, |frame| {
        let header = Rect::new(area.x, area.y, area.w, bottom);
        let covered = frame.covered(header, 0.0);
        frame.shadow(header, 0.0, 6.0, 0.0, p.shadow, Some(covered));
        // 外层固定行覆盖正在被推出的内层固定行。
        for row in rows.iter().rev() {
            let band = Rect::new(area.x, area.y + row.y, area.w, lh);
            frame.rect_clipped(band, p.editor_bg, 0.0, area);
            let target = DiffTarget::Sticky(side, row.line, row.depth);
            if app.hover == Hover::Diff(target) {
                frame.rect_clipped(band, p.hover, 0.0, area);
            }
            let baseline = baseline_in(band, app.mono_metrics.ascent, app.mono_metrics.descent);
            let number = (row.line + 1).to_string();
            frame.text(
                FontKind::Mono,
                app.fonts.code,
                pane.number_end - number.len() as f32 * app.cell,
                baseline,
                &number,
                p.gutter_text,
                pane.gutter,
            );
            line_chars(source, row.line, &mut chars);
            tokens.clear();
            if let Some(state) = source.highlighter.known_state(row.line) {
                winc_syntax::highlight_line(
                    source.highlighter.language(),
                    &chars,
                    state,
                    &mut tokens,
                );
            }
            draw_glyphs(
                app, frame, &chars, &tokens, origin_x, baseline, tab_width, pane.text,
            );
        }
    });

    let hit_width = (parts.body.right() - SCROLLBAR_HIT - area.x).clamp(0.0, area.w);
    rows.iter()
        .map(|row| {
            let band = Rect::new(area.x, area.y + row.y, hit_width, lh);
            (
                band.intersect(area),
                DiffTarget::Sticky(side, row.line, row.depth),
            )
        })
        .collect()
}
