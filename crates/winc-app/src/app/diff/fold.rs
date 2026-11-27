//! 差异标签中未改动内容的折叠与展开，以及折叠、展开时的过渡。

use super::*;

impl App {
    /// 开始第 `gap` 段未改动内容展开（`expanding` 为真）或折叠的过渡，返回是否已开始。
    ///
    /// 调用时差异中该段须为展开状态。以下情况不做过渡：动态效果已关闭；该段无法折叠为一个
    /// 连续的区间；该段的起点不在编辑区顶部之下（顶部的行需要保持不动）。
    pub(super) fn start_diff_fold(&mut self, gap: usize, expanding: bool) -> bool {
        (self.clock(FOLD_ANIMATION))
            .is_some_and(|clock| self.begin_diff_fold(gap, expanding, clock))
    }

    /// 同 [`Self::start_diff_fold`]，过渡按给定的时钟 `clock` 计时；测试中使用固定时刻的时钟。
    pub(in crate::app) fn begin_diff_fold(
        &mut self,
        gap: usize,
        expanding: bool,
        clock: Clock,
    ) -> bool {
        let Some(doc) = self.editor.active() else {
            return false;
        };
        let Some(view) = doc.diff.as_deref().filter(|view| view.is_open(gap)) else {
            return false;
        };
        let Some(lines) = view.fold_lines(gap) else {
            return false;
        };
        let rows = [LEFT, RIGHT]
            .map(|side| view.row_of(side, lines[side].start)..view.row_of(side, lines[side].end));
        let top = view.row_at(RIGHT, doc.scroll_line.max(0.0));
        if rows[RIGHT].start as f64 <= top || rows[LEFT].len() != rows[RIGHT].len() {
            return false;
        }
        let section = self.combined_inside_id();
        let Some(doc) = self.editor.active() else {
            return false;
        };
        self.motion.diff_fold = Some(DiffFoldMotion {
            doc: doc.id,
            section,
            gap,
            rows,
            expanding,
            scroll: doc.scroll_line,
            clock,
        });
        self.request_redraw();
        true
    }

    /// 结束进行中的展开或折叠过渡；折叠的过渡此时把折叠写入差异。
    ///
    /// `force` 为假时只在过渡的时钟走完后结束，每帧绘制之前调用；为真时立即结束，在修改折叠
    /// 状态或处理点击之前调用，使后续操作按最终的排版进行。
    pub(in crate::app) fn finish_diff_fold(&mut self, force: bool) {
        let Some(motion) = self.motion.diff_fold.take_if(|m| force || m.clock.done()) else {
            return;
        };
        if motion.expanding {
            return;
        }
        // 「未提交的更改」中的一段：换进那一段再折叠（已经换进来时按普通的差异处理）。
        if let Some(section) = motion.section
            && self.combined_inside_id() != Some(section)
        {
            if let Some(i) = self.combined_section_index(section) {
                self.with_combined_section(i, |app| {
                    app.refold_diff(|view, row| view.fold(motion.gap, row));
                });
            }
            return;
        }
        if self.editor.active().is_some_and(|d| d.id == motion.doc) {
            self.refold_diff(|view, row| view.fold(motion.gap, row));
        } else if let Some(view) = (self.editor.documents.iter_mut())
            .find(|d| d.id == motion.doc)
            .and_then(|d| d.diff.as_deref_mut())
        {
            view.fold(motion.gap, 0);
        }
    }

    /// 在差异标签中执行代码折叠命令 `command`；返回当前标签是否为差异标签。
    ///
    /// 光标在右侧工作区文件中时作用于右侧，否则作用于只读光标所在的一侧；尚未放置光标时作用于
    /// 右侧窗口顶部的代码。未改动长段的折叠仍由标题栏开关和折叠行单独控制。
    pub(in crate::app) fn fold_diff(&mut self, command: FoldCommand) -> bool {
        let Some(index) = self.editor.active else {
            return false;
        };
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return false;
        };
        let top = view.row_at(RIGHT, self.editor.documents[index].scroll_line.max(0.0)) as usize;
        let editing = view.editing;
        let source = editing.then(|| {
            let path = view.new.path.as_deref()?;
            let source = self
                .editor
                .documents
                .iter()
                .position(|doc| doc.path.as_deref() == Some(path))?;
            let doc = &self.editor.documents[source];
            Some((source, doc.text().pos_to_line_col(doc.selection.head).0))
        });
        let caret = source
            .flatten()
            .map(|(source, line)| (RIGHT, line, Some(source)))
            .or_else(|| {
                view.read_caret().map(|(side, pos)| {
                    let text = &view.side(side).text;
                    (side, text.char_to_line(pos.min(text.len_chars())), None)
                })
            });
        let (side, line, source) = caret.unwrap_or_else(|| {
            let line = view.rows[RIGHT]
                .iter()
                .skip(top)
                .map(|row| match row {
                    SideRow::Line { line, .. } => *line,
                    SideRow::Fold { lines, .. } => lines.start,
                })
                .next()
                .unwrap_or(0);
            (RIGHT, line, None)
        });
        let gap = view.gap_of(side, line);
        let open = gap.is_some_and(|gap| view.is_open(gap));
        if !self.fold_diff_code_at(side, line, source, command, top) {
            // 兼容差异原有的快捷键行为：当前位置没有可执行的代码折叠时，操作未改动长段。
            match (command, gap) {
                (FoldCommand::FoldAll, _) => self.refold_diff(DiffView::fold_all),
                (FoldCommand::UnfoldAll, _) => self.refold_diff(DiffView::unfold_all),
                (
                    FoldCommand::Fold | FoldCommand::FoldRecursively | FoldCommand::Toggle,
                    Some(gap),
                ) if open => self.press_diff(DiffTarget::Refold(gap)),
                (
                    FoldCommand::Unfold | FoldCommand::UnfoldRecursively | FoldCommand::Toggle,
                    Some(gap),
                ) => self.press_diff(DiffTarget::Fold(gap)),
                _ => {}
            }
        }
        true
    }

    /// 在差异一侧的指定行执行代码折叠，保持右侧顶部内容不动。
    pub(super) fn fold_diff_code_at(
        &mut self,
        side: usize,
        line: usize,
        source: Option<usize>,
        command: FoldCommand,
        top: usize,
    ) -> bool {
        self.finish_diff_fold(true);
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(view) = doc.diff.as_deref_mut() else {
            return false;
        };
        let (row, changed, visible) = view.run_code_fold(side, line, command, top, tab_width);
        if !changed {
            return false;
        }
        let row = view.pos_of(RIGHT, row as f64);
        if source.is_none()
            && view
                .read_selection
                .is_some_and(|selection| selection.side == side)
        {
            let pos = view.side(side).text.line_end(visible);
            view.read_selection = Some(ReadSelection {
                side,
                range: (pos, pos),
            });
        }
        set_scroll_line(doc, row);
        if let Some(source) = source {
            let source = &mut self.editor.documents[source];
            let pos = source.text().line_end(visible);
            source.collapse_selections();
            source.selection = Selection::point(pos);
        }
        self.clamp_scroll();
        self.request_redraw();
        true
    }

    /// 点击差异行号栏的代码折叠箭头或行尾标记。
    pub(super) fn toggle_diff_code_fold(&mut self, side: usize, line: usize) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        let top = view.row_at(RIGHT, doc.scroll_line.max(0.0)) as usize;
        self.fold_diff_code_at(side, line, None, FoldCommand::Toggle, top);
    }

    /// 以 `change` 修改当前差异标签的折叠状态，右侧顶部的行保持在顶部。
    ///
    /// `change` 的参数为差异视图和修改前右侧顶部的显示行，返回该行的内容在修改后所在的显示行。
    pub(super) fn refold_diff(&mut self, change: impl FnOnce(&mut DiffView, usize) -> usize) {
        self.finish_diff_fold(true);
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        if let Some(view) = doc.diff.as_deref_mut() {
            let row = view.row_at(RIGHT, doc.scroll_line.max(0.0)) as usize;
            let row = change(view, row);
            let row = view.pos_of(RIGHT, row as f64);
            set_scroll_line(doc, row);
        }
        self.clamp_scroll();
        self.request_redraw();
    }
}
