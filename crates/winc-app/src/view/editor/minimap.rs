//! 编辑区右侧的缩略图：每行代码画成一道彩色的小块，半透明的滑块标出当前显示的范围。
//!
//! 只绘制缩略图放得下的行，文档长于缩略图时，缩略图随滚动位置按比例移动。自动换行时每个显示行
//! 各占一道。

use std::ops::ControlFlow;

use winc_syntax::Token;

use super::*;
use crate::theme::MINIMAP_ROW;

/// 一列字符的宽度（逻辑像素，经验值）：约 100 列的代码放得下，长行不会在右边齐齐截断。
const CELL: f32 = 0.75;
/// 小块左边留出的空隙（逻辑像素）。
const PAD: f32 = 4.0;
/// 滑块的圆角半径（逻辑像素）。
const SLIDER_RADIUS: f32 = 3.0;
/// 一道小块的高度（逻辑像素），比一行略矮，相邻两行之间留出缝隙。
const STROKE: f32 = 1.5;
/// 小块的不透明度（经验值）：比正文淡，不与代码抢视线。
const OPACITY: f32 = 0.75;
/// 注释小块的不透明度（经验值）：更淡，一眼能分出代码和注释。
const COMMENT_OPACITY: f32 = 0.4;

/// 绘制缩略图；当前布局没有缩略图时什么也不做。
///
/// # Panics
///
/// `index` 超出 `documents` 的范围时 panic。
pub(super) fn draw_minimap(app: &mut App, frame: &mut Frame<'_>, index: usize) {
    let area = app.layout.minimap;
    if area.w <= 0.0 || area.h <= 0.0 {
        return;
    }
    let p = app.palette;
    let (first, slider, active) = app.minimap_view();
    let shown = (area.h / MINIMAP_ROW).ceil() as usize;
    let tab_width = app.editor.documents[index].tab_width(app.editor.tab_width);
    let doc = &mut app.editor.documents[index];
    let segments: Vec<_> = doc
        .folds()
        .segments(first, shown, doc.text().line_count())
        .into_iter()
        .collect();
    // 不额外推算高亮：跳到长文件靠后的位置时由编辑区分帧推算，缩略图先按已有的状态绘制。
    let states: Vec<_> = segments
        .iter()
        .map(|range| doc.highlight_state(range.start, 0))
        .collect();
    let doc = &app.editor.documents[index];
    let language = doc.highlighter.language();

    let color = if active { p.selected_inactive } else { p.hover };
    frame.rect_clipped(slider, color, SLIDER_RADIUS, area);

    // 光标所在行画在代码下面，标出光标的位置。
    let caret = doc.row_of(doc.text().line_of(doc.selection.head));
    let caret = Rect::new(
        area.x,
        area.y + (caret as f32 - first as f32) * MINIMAP_ROW,
        area.w,
        STROKE,
    );
    frame.rect_clipped(caret, p.accent.alpha(p.accent.a * 0.5), 0.0, area);

    let columns = ((area.w - PAD) / CELL) as usize;
    let (mut chars, mut tokens) = (Vec::new(), Vec::new());
    for (range, start) in segments.iter().zip(&states) {
        let mut state = start.unwrap_or(LineState::Normal);
        doc.text()
            .visit_lines(range.start, MAX_LINE_BYTES, |line, _, content| {
                let top = doc.row_of(line);
                let row_y = |row: usize| area.y + (row as f32 - first as f32) * MINIMAP_ROW;
                if line >= range.end || row_y(top) >= area.bottom() {
                    return ControlFlow::Break(());
                }
                chars.clear();
                chars.extend(content.chars().take(MAX_LINE_CHARS));
                state = winc_syntax::highlight_line(language, &chars, state, &mut tokens);
                let breaks = doc.line_breaks(line);
                for part in 0..breaks.rows() {
                    let y = row_y(top + part);
                    if y + STROKE <= area.y {
                        continue;
                    }
                    if y >= area.bottom() {
                        break;
                    }
                    let segment = (breaks.range(part, chars.len()), breaks.indent_of(part));
                    strokes(
                        (&chars, &tokens),
                        segment,
                        tab_width,
                        columns,
                        |cells, kind| {
                            let color = p.token(kind);
                            let rect = Rect::new(
                                area.x + PAD + cells.start as f32 * CELL,
                                y,
                                cells.len() as f32 * CELL,
                                STROKE,
                            );
                            let opacity = if kind == TokenKind::Comment {
                                COMMENT_OPACITY
                            } else {
                                OPACITY
                            };
                            frame.rect_clipped(rect, color.alpha(color.a * opacity), 0.0, area);
                        },
                    );
                }
                ControlFlow::Continue(())
            });
    }
}

/// 把一个显示行切成连续的小块，对每块调用 `f(列范围, 类别)`：空白不画，同一类别的相邻字符合成
/// 一块；只处理前 `columns` 列。
///
/// `line` 是整行的字符和词法单元，`segment` 是这个显示行的字符范围和缩进（列）。
fn strokes(
    (chars, tokens): (&[char], &[Token]),
    (range, indent): (std::ops::Range<usize>, usize),
    tab_width: usize,
    columns: usize,
    mut f: impl FnMut(std::ops::Range<usize>, TokenKind),
) {
    let mut run: Option<(std::ops::Range<usize>, TokenKind)> = None;
    let (mut cell, mut next) = (indent, 0);
    for (i, &c) in chars.iter().enumerate().take(range.end).skip(range.start) {
        if cell >= columns {
            break;
        }
        while next < tokens.len() && tokens[next].end <= i {
            next += 1;
        }
        let kind = tokens
            .get(next)
            .filter(|token| token.start <= i)
            .map_or(TokenKind::Text, |token| token.kind);
        let width = match c {
            '\t' => tab_width.max(1) - cell % tab_width.max(1),
            c if c.is_ascii() => 1,
            _ => 2,
        };
        let blank = c.is_whitespace();
        match &mut run {
            Some((cells, same)) if !blank && *same == kind && cells.end == cell => {
                cells.end = cell + width;
            }
            _ => {
                if let Some((cells, kind)) = run.take() {
                    f(cells, kind);
                }
                if !blank {
                    run = Some((cell..cell + width, kind));
                }
            }
        }
        cell += width;
    }
    if let Some((cells, kind)) = run {
        f(cells.start..cells.end.min(columns), kind);
    }
}

#[cfg(test)]
mod tests;
