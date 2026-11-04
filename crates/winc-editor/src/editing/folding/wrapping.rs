//! 自动换行：每行在哪里断开，显示行和文本位置之间的换算、上下移动和行首行尾。

use super::*;

/// 返回第 `line` 行按 `width` 个显示列断成几段的结果。
///
/// 过长的行只看开头 [`MAX_LINE_CHARS`] 个字符，其后的字符不画。
pub(crate) fn line_breaks(
    text: &impl Lines,
    line: usize,
    width: usize,
    tab_width: usize,
) -> Breaks {
    let content = text.line_content(line);
    let slice = content.slice();
    breaks_of(slice.len_chars(), || slice.chars(), width, tab_width)
}

/// 一行文字按 `width` 个显示列断成几段的结果：`chars` 每次调用都从行首起依次给出这一行的字符，
/// `len` 是这个字符数的上界。
///
/// 过长的行只看开头 [`MAX_LINE_CHARS`] 个字符。
pub(super) fn breaks_of<I: Iterator<Item = char>>(
    len: usize,
    chars: impl Fn() -> I,
    width: usize,
    tab_width: usize,
) -> Breaks {
    // 先做粗略估计：每个字符最宽 2 个显示列（Tab 按 Tab 宽度计），按这个上界估算仍然放得下的
    // 行不必逐字符测量。
    if len.saturating_mul(tab_width.max(2)) <= width {
        return Breaks::default();
    }
    // 上面的上界偏保守（Tab 宽度越大，估算越容易判为放不下），而多数行实际放得下：因此边走边量，
    // 确认放不下时才取出字符细算，避免为同一行建立两次字符表。
    let mut cells = 0;
    let fits = chars().take(MAX_LINE_CHARS).all(|c| {
        cells += winc_core::text::char_cells(c, cells, tab_width);
        cells <= width.max(1)
    });
    if fits {
        return Breaks::default();
    }
    let chars: Vec<char> = chars().take(MAX_LINE_CHARS).collect();
    wrap::breaks(&chars, width, tab_width)
}

/// 返回第 `line` 行按 `width` 个显示列断行后占的显示行数。
pub(super) fn line_rows(text: &impl Lines, line: usize, width: usize, tab_width: usize) -> usize {
    line_breaks(text, line, width, tab_width).rows()
}

impl Document {
    /// 设置自动换行并在需要时重新计算，返回设置后是否换行。
    ///
    /// `width` 是一行放得下的显示列数，`None` 表示不换行。宽度、Tab 宽度变化，或重新读取文件、
    /// 改名后整体重新计算。大文件和按列对齐的 CSV 不换行。
    pub fn ensure_wrap(&mut self, width: Option<usize>, tab_width: usize) -> bool {
        let width = width.filter(|_| self.can_fold() && self.columns(tab_width).is_none());
        let fresh = match (self.folds.wrap(), width) {
            (None, None) => true,
            (Some(w), Some(width)) => {
                w.width == width && w.tab_width == tab_width && w.revision == self.revision
            }
            _ => false,
        };
        if !fresh {
            let revision = self.revision;
            self.folds
                .set_wrap(&self.text, width.map(|w| (w, tab_width)), revision);
        }
        width.is_some()
    }

    /// 是否开着自动换行。
    pub fn is_wrapping(&self) -> bool {
        self.folds.is_wrapping()
    }

    /// 返回自动换行的（断行宽度，Tab 宽度），单位为显示列；不换行时返回 `None`。
    pub fn wrap_width(&self) -> Option<(usize, usize)> {
        self.folds.wrap().map(|w| (w.width, w.tab_width))
    }

    /// 返回第 `line` 行的断行结果；不换行或放得下时只有一段。
    ///
    /// 绘制时每个可见行要调用多次：记录里这一行没有被断开时直接返回，不再逐字符测量。记录与文字
    /// 不同步（版本对不上，等待 `ensure_wrap` 重算）时仍按当前文字计算。
    pub fn line_breaks(&self, line: usize) -> Breaks {
        match self.folds.wrap() {
            Some(w) if w.revision == self.revision && !w.breaks_line(line) => Breaks::default(),
            Some(w) if line < self.text.line_count() => {
                line_breaks(&self.text, line, w.width, w.tab_width)
            }
            _ => Breaks::default(),
        }
    }

    /// 返回第 `line` 行占的显示行数。
    pub fn rows_of(&self, line: usize) -> usize {
        self.folds.rows_of(line)
    }

    /// 返回第 `row` 个显示行对应的（行号，段号），均从 0 起；超出时返回最后一行的最后一段。
    pub fn line_and_part(&self, row: usize) -> (usize, usize) {
        let last = self.text.line_count().saturating_sub(1);
        match self.folds.line_and_part(row) {
            (line, part) if line <= last => (line, part),
            _ => (last, self.folds.rows_of(last).saturating_sub(1)),
        }
    }

    /// 返回位置 `pos`（字符偏移）所在的显示行号（从 0 起）。
    pub fn row_at(&self, pos: usize) -> usize {
        let (line, col) = self.text.pos_to_line_col(pos);
        self.folds.row_of(line) + self.line_breaks(line).row_of(col)
    }

    /// 返回第 `row` 个显示行里、距行首 `cell` 个显示列处的位置（字符偏移）；落在一个字符中间时
    /// 取较近的一侧。
    ///
    /// 不是一行最后一段时，最多到这一段的最后一个字符之前，否则光标会画到下一段的开头。
    pub fn pos_in_row(&self, row: usize, cell: usize, tab_width: usize) -> usize {
        let (line, part) = self.line_and_part(row);
        let breaks = self.line_breaks(line);
        let content = self.text.line_content(line);
        let slice = content.slice();
        let len = slice.len_chars().min(MAX_LINE_CHARS);
        let range = breaks.range(part, len);
        let chars: Vec<char> = slice.chars().skip(range.start).take(range.len()).collect();
        let last = part + 1 == breaks.rows();
        let mut at = breaks.indent_of(part);
        let mut col = range.start;
        while col < range.end {
            let w = winc_core::text::char_cells(chars[col - range.start], at, tab_width);
            if at + w > cell {
                if cell - at.min(cell) > (at + w - cell) && (last || col + 1 < range.end) {
                    col += 1;
                }
                break;
            }
            at += w;
            col += 1;
        }
        if !last {
            col = col.min(range.end.saturating_sub(1)).max(range.start);
        }
        let pos = self.text.line_col_to_pos(line, col);
        winc_core::movement::snap_to_grapheme(&self.text, pos)
    }

    /// 自动换行时上下移动 `delta` 个显示行，返回（新位置（字符偏移），目标显示列 `goal`），
    /// 保持在同一显示列上；不换行时返回 `None`。
    pub(crate) fn wrapped_vertical(&self, delta: isize) -> Option<(usize, usize)> {
        let wrap = self.folds.wrap()?;
        let tab_width = wrap.tab_width;
        let head = self.selection.head;
        let (line, col) = self.text.pos_to_line_col(head);
        let breaks = self.line_breaks(line);
        let content = self.text.line_content(line);
        let goal = self
            .goal_cell
            .unwrap_or_else(|| wrap::cell_in_row(content.slice(), &breaks, col, tab_width));
        let row = self.folds.row_of(line) as isize + breaks.row_of(col) as isize + delta;
        if row < 0 {
            return Some((0, goal));
        }
        if row as usize >= self.row_count() {
            return Some((self.text.end_pos(), goal));
        }
        Some((self.pos_in_row(row as usize, goal, tab_width), goal))
    }

    /// 返回自动换行时「行首」「行尾」命令（`end` 为 `true` 时是行尾）的目标位置（字符偏移）。
    ///
    /// 目标先取当前这一段的头尾，已经在那里时取整行的头尾；应由普通的行首、行尾命令处理时返回
    /// `None`。
    pub(crate) fn wrapped_home_end(&self, end: bool) -> Option<usize> {
        self.folds.wrap()?;
        let head = self.selection.head;
        let (line, col) = self.text.pos_to_line_col(head);
        let breaks = self.line_breaks(line);
        if breaks.rows() == 1 {
            return None;
        }
        let len = self
            .text
            .line_content(line)
            .slice()
            .len_chars()
            .min(MAX_LINE_CHARS);
        let part = breaks.row_of(col);
        let range = breaks.range(part, len);
        let start = self.text.line_start(line);
        if end {
            // 最后一段按普通行尾处理。其他段停在最后一个字符之前（再往后会画到下一段开头），
            // 已经在那里时才到行尾。
            if part + 1 == breaks.rows() {
                return None;
            }
            let stop = range.end - 1;
            return Some(if col == stop {
                self.text.line_end(line)
            } else {
                start + stop
            });
        }
        (part > 0 && col != range.start).then_some(start + range.start)
    }
}
