//! 选区、光标和配对的括号。

use super::*;

/// 返回第 `line` 行的行末位置（字符偏移）。
///
/// 截断的行只读取了开头一段（`shown` 个字符），行末要由下一行的行首推出。
pub(super) fn line_end(
    text: &impl Lines,
    line: usize,
    line_start: usize,
    shown: usize,
    truncated: bool,
) -> usize {
    match truncated {
        false => line_start + shown,
        true if line + 1 < text.line_count() => text.line_start(line + 1) - 1,
        true => text.end_pos(),
    }
}

/// 返回选区 `selection` 涉及的行（左闭右开）。
///
/// 这几行以外的行上没有选中的部分，也不影响块的圆角，
/// 绘制选区时只量这几行：有成千上万个选区时，不必为每个选区把屏幕上的每一行都量一遍。
fn selection_lines(text: &impl Lines, selection: Selection) -> Range<usize> {
    text.pos_to_line_col(selection.from()).0..text.pos_to_line_col(selection.to()).0 + 1
}

/// 把行范围 `range` 限制在 `lines` 之内。
fn clamp_lines(range: &Range<usize>, lines: &Range<usize>) -> Range<usize> {
    range.start.max(lines.start)..range.end.min(lines.end)
}

/// 绘制自动换行时的选区 `selection`：每一段一块，相邻的块连成一片。
///
/// # Panics
///
/// `app` 没有当前文档时 panic。
pub(super) fn draw_wrapped_selection(
    app: &App,
    frame: &mut Frame<'_>,
    selection: Selection,
    segments: &[Range<usize>],
    origin_x: f32,
    row_y: &dyn Fn(usize) -> f32,
) {
    let doc = app.editor.active().expect("调用方保证有当前文档");
    let text = doc.text();
    let (cell, tab_width, clip, lh) = (
        app.cell,
        doc.tab_width(app.editor.tab_width),
        app.layout.text,
        app.fonts.line_height,
    );
    // 屏幕上从上到下的每一段：（y，选中部分的 x 范围）。
    let mut rows: Vec<(f32, Option<(f32, f32)>)> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    let lines = selection_lines(text, selection);
    for range in segments.iter().map(|r| clamp_lines(r, &lines)) {
        if range.is_empty() {
            continue;
        }
        text.visit_lines(range.start, MAX_LINE_BYTES, |line, line_start, content| {
            if line >= range.end {
                return ControlFlow::Break(());
            }
            chars.clear();
            chars.extend(content.chars().take(MAX_LINE_CHARS + 1));
            let truncated = chars.len() > MAX_LINE_CHARS;
            let len = chars.len().min(MAX_LINE_CHARS);
            let line_end = line_end(text, line, line_start, len, truncated);
            let breaks = doc.line_breaks(line);
            let y = row_y(line);
            for part in 0..breaks.rows() {
                let r = breaks.range(part, len);
                let (a, b) = (line_start + r.start, line_start + r.end);
                let last = part + 1 == breaks.rows();
                // 该段的末尾：最后一段为行尾，其他段到下一段的开头（不含）。
                let end = if last { line_end } else { b };
                let x_at = |pos: usize| {
                    let col = (pos - line_start).min(len);
                    let span = &chars[r.start..col.max(r.start)];
                    origin_x
                        + text::slice_cells(span, breaks.indent_of(part), tab_width) as f32 * cell
                };
                // 最后一段按行尾判断；其他段只看该段内的字符。
                let covered = if last {
                    selection.from() <= end && selection.to() > a
                } else {
                    selection.from() < end && selection.to() > a
                };
                let span = covered.then(|| {
                    let x0 = x_at(selection.from().max(a));
                    let mut x1 = x_at(selection.to().min(end));
                    // 选区跨过行尾时多绘制一小截（0.6 个字符宽），表示换行符也被选中。
                    if last && selection.to() > end {
                        x1 += cell * 0.6;
                    }
                    (x0, x1.max(x0 + 1.0))
                });
                rows.push((y + part as f32 * lh, span));
            }
            ControlFlow::Continue(())
        });
    }

    let color = selection_color(app);
    let covers = |other: Option<(f32, f32)>, x: f32| {
        other.is_some_and(|(a, b)| a <= x + 0.5 && b >= x - 0.5)
    };
    for (i, &(y, span)) in rows.iter().enumerate() {
        let Some((x0, x1)) = span else { continue };
        let above = i.checked_sub(1).and_then(|j| rows[j].1);
        let below = rows.get(i + 1).and_then(|r| r.1);
        let radius = |covered: bool| if covered { 0.0 } else { RADIUS_SELECTION };
        let radii = [
            radius(covers(above, x0 + 1.0)),
            radius(covers(above, x1 - 1.0)),
            radius(covers(below, x1 - 1.0)),
            radius(covers(below, x0 + 1.0)),
        ];
        frame.rect_corners(Rect::new(x0, y, x1 - x0, lh), color, radii, clip);
    }
}

/// 返回选区的底色；编辑区没有键盘焦点时使用较淡的 `Palette::selection_inactive`。
fn selection_color(app: &App) -> Color {
    if app.editor_focused() {
        app.palette.selection
    } else {
        app.palette.selection_inactive
    }
}

/// 绘制选区 `selection`：每行一块，跨行时相邻的块连成一片，只有露在外面的角是圆的。
///
/// `columns` 是 CSV 按列对齐时各列的位置（调用方每帧取一次）。
///
/// # Panics
///
/// `app` 没有当前文档时 panic。
pub(super) fn draw_selection(
    app: &App,
    frame: &mut Frame<'_>,
    selection: Selection,
    segments: &[Range<usize>],
    columns: Option<&Columns>,
    origin_x: f32,
    row_y: &dyn Fn(usize) -> f32,
) {
    let doc = app.editor.active().expect("调用方保证有当前文档");
    let text = doc.text();
    let (cell, tab_width, clip) = (
        app.cell,
        doc.tab_width(app.editor.tab_width),
        app.layout.text,
    );
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return;
    };

    // 上下各多取一行可见行，用来判断首尾两行的角是否需要圆角。
    let mut ranges = Vec::with_capacity(segments.len() + 2);
    // 第一个显示行之前没有可见行时（它是第 0 个显示行），不取上一行。
    if let Some(row) = doc.row_of(first.start).checked_sub(1) {
        let above = doc.line_of_row(row);
        ranges.push(above..above + 1);
    }
    ranges.extend(segments.iter().cloned());
    let below = doc.folds().visible_from(last.end);
    if below < text.line_count() {
        ranges.push(below..below + 1);
    }

    let mut spans: Vec<(usize, Option<(f32, f32)>)> = Vec::new();
    // CSV 按列对齐时各字符的显示列：各行共用一个缓冲，只排到选区在这一行的终点为止。
    let mut cells = Vec::new();
    let lines = selection_lines(text, selection);
    for range in ranges.iter().map(|r| clamp_lines(r, &lines)) {
        if range.is_empty() {
            continue;
        }
        text.visit_lines(range.start, MAX_LINE_BYTES, |line, line_start, content| {
            if line >= range.end {
                return ControlFlow::Break(());
            }

            let shown = content.chars().take(MAX_LINE_CHARS + 1).count();
            let truncated = shown > MAX_LINE_CHARS;
            let line_end = line_end(text, line, line_start, shown.min(MAX_LINE_CHARS), truncated);

            let span = (selection.from() <= line_end && selection.to() > line_start).then(|| {
                let slice = RopeSlice::from(content);
                if let Some(c) = columns {
                    let to = selection.to().min(line_end) - line_start;
                    c.layout(slice, tab_width, to, &mut cells, None);
                }
                let x_of = |pos: usize| {
                    let col = pos - line_start;
                    let cells = match columns {
                        Some(_) => cells[col.min(cells.len() - 1)],
                        None => text::col_to_cell(slice, col, tab_width),
                    };
                    origin_x + cells as f32 * cell
                };
                let x0 = x_of(selection.from().max(line_start));
                let mut x1 = x_of(selection.to().min(line_end));

                // 选区跨过行尾时多绘制一小截（0.6 个字符宽），表示换行符也被选中。
                if selection.to() > line_end {
                    x1 += cell * 0.6;
                }

                (x0, x1)
            });
            spans.push((line, span));
            ControlFlow::Continue(())
        });
    }

    // 某个角被相邻行的选区覆盖时不绘制圆角。
    let color = selection_color(app);
    let covers = |other: Option<(f32, f32)>, x: f32| {
        other.is_some_and(|(a, b)| a <= x + 0.5 && b >= x - 0.5)
    };

    for (i, &(line, span)) in spans.iter().enumerate() {
        let Some((x0, x1)) = span else { continue };

        if !segments.iter().any(|r| r.contains(&line)) {
            continue;
        }

        let above = i.checked_sub(1).and_then(|j| spans[j].1);
        let below = spans.get(i + 1).and_then(|s| s.1);
        let radius = |covered: bool| if covered { 0.0 } else { RADIUS_SELECTION };
        let radii = [
            radius(covers(above, x0 + 1.0)),
            radius(covers(above, x1 - 1.0)),
            radius(covers(below, x1 - 1.0)),
            radius(covers(below, x0 + 1.0)),
        ];
        let rect = Rect::new(x0, row_y(line), x1 - x0, app.fonts.line_height);
        frame.rect_corners(rect, color, radii, clip);
    }
}

/// 绘制光标，或输入法正在组字的文本；返回输入法候选框应对齐的矩形。
pub(in crate::view) fn draw_caret(
    app: &App,
    frame: &mut Frame<'_>,
    x: f32,
    y: f32,
    baseline: f32,
    clip: Rect,
) -> Rect {
    let p = app.palette;
    let (cell, tab_width, preedit) = (app.cell, app.editor.active_tab_width(), &app.preedit);

    if preedit.is_empty() {
        caret_bar(app, frame, x, y, clip);
        return Rect::new(x, y, 2.0, app.fonts.line_height);
    }

    let cells = |c: char| text::char_cells(c, 0, tab_width).max(1);
    let width = preedit.chars().map(cells).sum::<usize>() as f32 * cell;
    frame.rect_clipped(
        Rect::new(x, y, width, app.fonts.line_height),
        p.editor_bg,
        0.0,
        clip,
    );
    let mut px = x;

    for c in preedit.chars() {
        frame.cell_char(
            FontKind::Mono,
            app.fonts.code,
            px,
            baseline,
            cell,
            cells(c),
            c,
            p.text,
            clip,
        );
        px += cells(c) as f32 * cell;
    }

    frame.rect_clipped(
        Rect::new(x, baseline + 3.0, width, 1.0),
        p.accent,
        0.0,
        clip,
    );
    Rect::new(px, y, 2.0, app.fonts.line_height)
}

/// 在一行的顶边 `y` 处、横坐标 `x` 绘制一道光标竖线；闪烁到暗的那一半时不绘制。
pub(in crate::view) fn caret_bar(app: &App, frame: &mut Frame<'_>, x: f32, y: f32, clip: Rect) {
    if app.caret_visible {
        let bar = Rect::new(x.round(), y + 1.0, 2.0, app.fonts.line_height - 2.0);
        frame.rect_clipped(bar, app.palette.caret, 1.0, clip);
    }
}

/// 返回光标旁括号及与之配对的括号的位置；没有时返回 `None`。
///
/// 结果按（文档、版本号、光标位置）缓存：在大文件中向前查找需要读取多行，光标不动时不必每帧重算。
pub(super) fn matching_brace(app: &mut App, index: usize) -> Option<(usize, usize)> {
    let doc = &app.editor.documents[index];
    let key = BraceKey {
        doc: doc.id,
        revision: doc.revision,
        head: doc.selection.head,
    };

    if let Some((cached, found)) = app.brace_cache
        && cached == key
    {
        return found;
    }

    let found = movement::matching_bracket(doc.text(), doc.selection.head, BRACE_SCAN_LIMIT);
    app.brace_cache = Some((key, found));
    found
}
