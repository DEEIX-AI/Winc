//! 模态编辑层在 [`Document`] 上需要的接口：撤销分组、按位置给出的修改、折叠与显示行的查询。
//!
//! 模态编辑层自己计算修改的范围与修改之后的光标，不经 [`Command`]；这里把修改组成事务交给
//! [`Document::apply`]，并提供按文本行之外的单位（已折叠的区域、显示行）查询位置的方法。

use std::ops::Range;

use winc_core::edit::EditKind;
use winc_core::{Change, Lines, Selection, Transaction};

use crate::{Command, Document, EDIT_LIMIT};

impl Document {
    /// 开始一个撤销分组：到对应的 [`Document::end_undo_group`] 为止的修改合并成一个撤销单元。
    pub fn begin_undo_group(&mut self) {
        self.history.begin_group();
    }

    /// 结束最近一次 [`Document::begin_undo_group`] 开始的分组。
    pub fn end_undo_group(&mut self) {
        self.history.end_group();
    }

    /// 返回下一次重做的撤销单元执行之前光标所在的位置；没有可重做的内容时返回 `None`。
    pub fn redo_origin(&self) -> Option<usize> {
        self.history.redo_origin().map(|selection| selection.head)
    }

    /// 是否可以读取并修改位置区间 `from..to` 内的文本：区间的字节数不超过 [`EDIT_LIMIT`]。
    pub fn fits_edit(&self, from: usize, to: usize) -> bool {
        self.text.byte_len(from.min(to), to) <= EDIT_LIMIT
    }

    /// 依次应用 `changes`，光标设在 `after`，撤销之后光标位于 `before`；返回是否已应用。
    ///
    /// 每处修改的起点是应用之前各处修改后的位置。文档只读，或被删除的文本合计超过
    /// [`EDIT_LIMIT`] 时不应用。其余选区（多光标）被收回。
    pub fn apply_changes(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool {
        let removed: usize = changes.iter().map(|change| change.removed.len()).sum();
        if self.readonly || removed as u64 > EDIT_LIMIT {
            return false;
        }
        self.collapse_selections();
        self.goal_cell = None;
        self.apply(Transaction::new(
            changes,
            Selection::point(before),
            Selection::point(after),
            EditKind::Other,
        ));
        true
    }

    /// 在位置 `at` 换行并把光标放在新行上，缩进规则与 [`Command::Newline`] 相同；撤销之后
    /// 选区回到调用之前的状态。
    pub fn open_line_at(&mut self, at: usize, tab_width: usize) {
        if self.readonly {
            return;
        }
        let original = self.selection;
        self.collapse_selections();
        self.selection = Selection::point(at.min(self.text.end_pos()));
        self.goal_cell = None;
        match self.transaction(&Command::Newline, tab_width) {
            Some(mut transaction) => {
                transaction.before = original;
                self.apply(transaction);
            }
            None => self.selection = original,
        }
    }

    /// 切换第 `first` 至 `last` 行的行注释，返回是否有修改；选区在修改之后收回为光标。
    pub fn toggle_comment_lines(&mut self, first: usize, last: usize, tab_width: usize) -> bool {
        if self.readonly {
            return false;
        }
        let original = self.selection;
        self.collapse_selections();
        let last = last.min(self.text.line_count().saturating_sub(1));
        self.selection = Selection::new(self.text.line_start(first), self.text.line_end(last));
        let Some(mut transaction) = self.transaction(&Command::ToggleComment, tab_width) else {
            self.selection = original;
            return false;
        };
        transaction.before = Selection::point(original.head);
        transaction.after = Selection::point(self.text.line_start(first));
        self.apply(transaction);
        true
    }

    /// 是否存在已折叠的块。
    pub fn has_collapsed_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    /// 返回包含第 `line` 行的已折叠的块的首行与末行；嵌套时取最外层，不在折叠中时返回 `None`。
    pub fn collapsed_fold_at(&self, line: usize) -> Option<(usize, usize)> {
        (self.folds.iter())
            .filter(|fold| fold.start <= line && line <= fold.end)
            .map(|fold| (fold.start, fold.end))
            .reduce(|(first, last), (start, end)| (first.min(start), last.max(end)))
    }
}

impl Document {
    /// 返回位置 `pos` 所在显示行的位置范围；不换行或该行只有一个显示行时为整行。
    pub fn display_row_span(&self, pos: usize) -> Range<usize> {
        let (line, col) = self.text.pos_to_line_col(pos);
        let start = self.text.line_start(line);
        if self.folds.wrap().is_none() || self.rows_of(line) == 1 {
            return start..self.text.line_end(line);
        }
        let row = self.folds.row_of(line) + self.part_of(line, col);
        let (first, last) = self.row_bounds(row);
        first..last
    }
}

#[cfg(test)]
mod tests;
