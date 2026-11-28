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

    /// 在差异标签中执行折叠命令 `command`；返回当前标签是否为差异标签。
    ///
    /// 命令作用于代码块，与编辑器相同：光标在右侧工作区文件中时作用于右侧，否则作用于只读光标
    /// 所在的一侧，尚未放置光标时作用于右侧窗口顶部的代码。「全部折叠」「全部展开」同时收起、
    /// 展开未改动的长段。
    pub(in crate::app) fn fold_diff(&mut self, command: FoldCommand) -> bool {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return false;
        };
        let source = view.editing.then(|| {
            let path = view.new.path.as_deref()?;
            let source =
                (self.editor.documents.iter()).position(|doc| doc.path.as_deref() == Some(path))?;
            let doc = &self.editor.documents[source];
            Some((source, doc.text().pos_to_line_col(doc.selection.head).0))
        });
        let caret = (source
            .flatten()
            .map(|(source, line)| (RIGHT, line, Some(source))))
        .or_else(|| {
            view.read_caret().map(|(side, pos)| {
                let text = &view.side(side).text;
                (side, text.char_to_line(pos.min(text.len_chars())), None)
            })
        });
        let (side, line, source) = caret.unwrap_or_else(|| {
            let top = self.editor.active().map_or(0.0, |d| d.scroll_line.max(0.0));
            let top = view.row_at(RIGHT, top) as usize;
            let line = (view.rows[RIGHT].iter().skip(top))
                .map(|row| match row {
                    SideRow::Line { line, .. } => *line,
                    SideRow::Fold { lines, .. } => lines.start,
                })
                .next()
                .unwrap_or(0);
            (RIGHT, line, None)
        });
        self.change_diff_code_folds(source, |view, top, tab_width| {
            view.run_code_fold((side, line), command, top, tab_width)
        });
        match command {
            FoldCommand::FoldAll => self.refold_diff(DiffView::fold_all),
            FoldCommand::UnfoldAll => self.refold_diff(DiffView::unfold_all),
            _ => {}
        }
        true
    }

    /// 点击差异第 `side` 侧第 `line` 行的代码折叠箭头或收起标记。
    pub(super) fn toggle_diff_code_fold(&mut self, side: usize, line: usize) {
        self.change_diff_code_folds(None, |view, top, tab_width| {
            view.toggle_code_fold_at((side, line), top, tab_width)
        });
    }

    /// 以 `change` 修改当前差异标签的代码折叠，右侧顶部的内容保持在顶部。
    ///
    /// `change` 的参数为差异视图、修改前右侧顶部的显示行和 Tab 宽度，返回值见
    /// [`DiffView::run_code_fold`]。光标所在的行被收起时停在收起后显示的行末：`source` 是右侧
    /// 对应的文档时移动该文档的光标，否则移动差异的只读光标。
    fn change_diff_code_folds(
        &mut self,
        source: Option<usize>,
        change: impl FnOnce(&mut DiffView, usize, usize) -> (usize, bool, usize),
    ) {
        self.finish_diff_fold(true);
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let top = doc.scroll_line.max(0.0);
        let Some(view) = doc.diff.as_deref_mut() else {
            return;
        };
        let top = view.row_at(RIGHT, top) as usize;
        let (row, changed, visible) = change(view, top, tab_width);
        if !changed {
            return;
        }
        // 只读光标所在的行被收起时（任一侧的折叠都可能隐藏该行），移到收起后显示的行末。
        if let Some((side, pos)) = view.read_caret() {
            let text = &view.side(side).text;
            let line = text.char_to_line(pos.min(text.len_chars()));
            let shown = view.visible_code_line(side, line);
            if shown != line {
                let pos = text.line_end(shown);
                view.read_selection = Some(ReadSelection {
                    side,
                    range: (pos, pos),
                });
            }
        }
        let row = view.pos_of(RIGHT, row as f64);
        set_scroll_line(doc, row);
        if let Some(source) = source {
            let source = &mut self.editor.documents[source];
            if source.text().pos_to_line_col(source.selection.head).0 != visible {
                let pos = source.text().line_end(visible);
                source.collapse_selections();
                source.selection = Selection::point(pos);
            }
        }
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 使所有已打开和之后打开的差异按设置（`diffCollapseUnchanged`）折叠或展开未更改的内容：
    /// 标题栏上的开关和设置是同一项。各标签顶部的行保持在顶部。
    pub(in crate::app) fn apply_diff_collapse(&mut self) {
        let on = self.config.diff_collapse_unchanged != Some(false);
        self.finish_diff_fold(true);
        self.editor.diff_collapse = on;
        for doc in &mut self.editor.documents {
            let Some(view) = doc.diff.as_deref_mut() else {
                continue;
            };
            if view.collapses() == on {
                continue;
            }
            let row = view.row_at(RIGHT, doc.scroll_line.max(0.0)) as usize;
            let row = view.set_collapse(on, row);
            let scroll = view.pos_of(RIGHT, row as f64);
            set_scroll_line(doc, scroll);
        }
        self.apply_combined_collapse(on);
        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
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
