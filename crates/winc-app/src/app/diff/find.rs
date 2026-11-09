//! 只读差异快照的查找。匹配属于这一侧的文本，不涉及工作区文件。

use super::*;

impl App {
    /// 右侧没有关联工作区文件的差异，例如从 Git 日志打开的提交。
    pub(crate) fn readonly_diff(&self) -> Option<&DiffView> {
        self.editor
            .active()?
            .diff
            .as_deref()
            .filter(|view| view.new.path.is_none())
    }

    /// 优先查光标所在的一侧，尚未点过时查右侧。已删除的文件只有左侧。
    pub(in crate::app) fn update_diff_find(&mut self) -> Option<(DocumentId, Selection)> {
        let view = self.readonly_diff()?;
        let side = view
            .read_selection
            .map(|s| s.side)
            .filter(|&side| !view.side(side).missing)
            .unwrap_or(if view.new.missing { LEFT } else { RIGHT });
        let selection = view
            .read_selection
            .filter(|s| s.side == side)
            .map_or(Selection::default(), |s| {
                Selection::new(s.range.0, s.range.1)
            });
        let doc = self.editor.active()?;
        let text = &doc.diff.as_deref()?.side(side).text;
        if self.search.snapshot_side(doc) != Some(side) {
            self.pending_jump = None;
        }
        self.search.update_snapshot(doc, side, text);
        Some((doc.id, selection))
    }

    /// 定位到只读快照的匹配，并展开挡住匹配的未改动段。
    pub(in crate::app) fn reveal_diff_match(&mut self, side: usize, start: usize, end: usize) {
        let tab_width = self.editor.tab_width;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(view) = doc.diff.as_deref_mut() else {
            return;
        };
        let text = &view.side(side).text;
        let (line, col) = text.pos_to_line_col(start);
        let last_line = text.line_of(end.saturating_sub(1).max(start));
        let parts = diff_parts(self.layout.editor, view, self.cell);
        let pane = parts.panes[side].text;
        let x = winc_core::text::col_to_cell(text.line(line), col, tab_width) as f32 * self.cell;
        let visible = parts.body.h / self.fonts.line_height;
        let covered = if self.layout.search.x < pane.right() && self.layout.search.h > 0.0 {
            (self.layout.search.bottom() - parts.body.y) / self.fonts.line_height
        } else {
            0.0
        };

        view.editing = false;
        view.read_selection = Some(ReadSelection {
            side,
            range: (start, end),
        });
        view.reveal_line(side, line);
        view.reveal_line(side, last_line);
        let row = view.row_of_col(side, line, col) as f32;
        let (row, top) = if view.is_unified() {
            (view.pos_of(side, row), doc.scroll_line)
        } else {
            (row, view.side_scroll(side, doc.scroll_line, visible))
        };
        if row < top + covered || row + 1.0 > top + visible {
            let top = (row - visible / 2.0).max(0.0);
            doc.scroll_line = if view.is_unified() {
                top
            } else {
                view.pos_of_top(side, top, visible)
            };
            view.focus = None;
        }
        if !view.is_wrapping() {
            let width = (pane.w - 2.0 * DIFF_TEXT_PAD).max(self.cell);
            if x < doc.scroll_x || x + self.cell > doc.scroll_x + width {
                doc.scroll_x = (x - width / 2.0).max(0.0);
            }
        }
        self.clamp_scroll();
        self.request_redraw();
    }
}
