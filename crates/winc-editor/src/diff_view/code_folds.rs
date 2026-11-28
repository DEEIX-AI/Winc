//! 差异中的代码折叠：按语法收起一侧的代码块，另一侧同步收起对应的内容。
//!
//! 并排显示时，两次改动之间未改动的内容两侧逐行对应，滚动按此对齐。只收起一侧会让两侧错开，
//! 因此每个折叠在两侧各隐藏一段：块所在一侧隐藏块的首行之后到末行，另一侧隐藏与之对应的行。

use std::ops::Range;
use std::sync::Arc;

use super::{DiffSide, DiffView, LEFT, RIGHT, SideRow};
use crate::FoldCommand;
use crate::document::rope_starts_quoted;
use crate::editing::scopes::{self, FoldRange};

/// 收起的一个代码块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CodeFold {
    /// 块所在的一侧（[`LEFT`] 或 [`RIGHT`]）。
    side: usize,
    /// 块的首行，收起后仍显示。
    start: usize,
    /// 块的末行，收起后不显示。
    end: usize,
}

/// 一侧的行边界（第 `b` 行之前的位置）在改动中间时映射到另一侧的哪一端。
#[derive(Clone, Copy)]
enum Edge {
    /// 隐藏范围的起点：取另一侧这段改动之后，不隐藏其中的行。
    Start,
    /// 隐藏范围的终点：取另一侧这段改动之前，不隐藏其中的行。
    End,
}

impl DiffSide {
    /// 返回这一侧全部可折叠的代码块，按（文字版本号，Tab 宽度）缓存。
    fn fold_ranges(&mut self, tab_width: usize) -> Arc<[FoldRange]> {
        let key = (self.revision, tab_width);
        if let Some((revision, tab, ranges)) = &self.fold_cache
            && (*revision, *tab) == key
        {
            return Arc::clone(ranges);
        }
        // 先把高亮状态推进到末尾，才能知道哪些行在多行字符串里。
        let last = self.text.len_lines().saturating_sub(1);
        self.highlighter.state_at(&self.text, last, usize::MAX);
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| rope_starts_quoted(highlighter, text, line);
        let ranges: Arc<[FoldRange]> =
            scopes::fold_ranges(text, language, tab_width, &mut quoted).into();
        self.fold_cache = Some((key.0, key.1, Arc::clone(&ranges)));
        ranges
    }

    /// 返回 `lines`（升序）中是代码块开头的行；只检查给出的行，供绘制屏幕上的折叠箭头。
    fn block_starts(
        &mut self,
        lines: impl IntoIterator<Item = usize>,
        tab_width: usize,
    ) -> Vec<usize> {
        let count = self.text.len_lines();
        let language = self.highlighter.language();
        let (text, highlighter) = (&self.text, &mut self.highlighter);
        let mut quoted = |line| rope_starts_quoted(highlighter, text, line);
        let lines = lines.into_iter().take_while(|&line| line < count);
        scopes::block_starts(text, language, lines, tab_width, &mut quoted)
    }
}

impl DiffView {
    /// 返回第 `side` 侧第 `line` 行下面收起的代码块在这一侧隐藏的行；该行下面没有收起的块时
    /// 返回 `None`。
    ///
    /// 块所在的一侧是块的首行，另一侧是隐藏内容之前的那一行，两侧都在这里显示收起标记。
    pub fn collapsed_code_fold(&self, side: usize, line: usize) -> Option<Range<usize>> {
        (self.code_folds.iter())
            .map(|fold| self.hidden_by(fold)[side].clone())
            .find(|hidden| !hidden.is_empty() && hidden.start == line + 1)
    }

    /// 返回第 `side` 侧 `lines`（升序）中是可折叠代码块开头的行。
    pub fn code_fold_starts(
        &mut self,
        side: usize,
        lines: impl IntoIterator<Item = usize>,
        tab_width: usize,
    ) -> Vec<usize> {
        self.side_mut(side).block_starts(lines, tab_width)
    }

    /// 在第 `side` 侧的第 `line` 行执行代码折叠命令 `command`。
    ///
    /// `row` 是修改前右侧顶部的显示行。返回（该内容修改后的右侧显示行、状态是否改变、`line`
    /// 修改后实际显示的行）。[`FoldCommand::UnfoldAll`] 展开两侧的全部代码块，其余命令作用于
    /// 这一侧的块；`line` 下面是另一侧收起的块时，展开和切换作用于该块。
    pub fn run_code_fold(
        &mut self,
        (side, line): (usize, usize),
        command: FoldCommand,
        row: usize,
        tab_width: usize,
    ) -> (usize, bool, usize) {
        let anchor = self.anchor_line(row);
        let before = self.code_folds.clone();
        let header = (self.code_folds.iter().copied())
            .find(|fold| self.hidden_by(fold)[side].start == line + 1 && fold.side != side);
        match (command, header) {
            (FoldCommand::UnfoldAll, _) => self.code_folds.clear(),
            (FoldCommand::Unfold | FoldCommand::Toggle, Some(header)) => {
                self.code_folds.retain(|fold| *fold != header);
            }
            _ => {
                let ranges = self.side_mut(side).fold_ranges(tab_width);
                let folded = |start| self.code_folds.contains(&fold_of(side, start, &ranges));
                let (fold, unfold) = command.plan(&ranges, line, folded);
                self.code_folds
                    .retain(|f| !(f.side == side && unfold.iter().any(|r| r.start == f.start)));
                for range in fold {
                    let fold = CodeFold {
                        side,
                        start: range.start,
                        end: range.end,
                    };
                    if !self.code_folds.contains(&fold) {
                        self.code_folds.push(fold);
                    }
                }
            }
        }
        self.finish_code_fold(before, anchor, side, line)
    }

    /// 点击第 `side` 侧第 `line` 行的折叠箭头或收起标记：该行下面有收起的块时展开，该行是一个
    /// 块的开头时收起。返回值同 [`Self::run_code_fold`]。
    pub fn toggle_code_fold_at(
        &mut self,
        (side, line): (usize, usize),
        row: usize,
        tab_width: usize,
    ) -> (usize, bool, usize) {
        let anchor = self.anchor_line(row);
        let before = self.code_folds.clone();
        let header = (self.code_folds.iter().copied())
            .find(|fold| self.hidden_by(fold)[side].start == line + 1);
        match header {
            Some(header) => self.code_folds.retain(|fold| *fold != header),
            None => {
                let ranges = self.side_mut(side).fold_ranges(tab_width);
                if let Some(range) = ranges.iter().find(|r| r.start == line) {
                    self.code_folds.push(CodeFold {
                        side,
                        start: range.start,
                        end: range.end,
                    });
                }
            }
        }
        self.finish_code_fold(before, anchor, side, line)
    }

    /// 返回第 `side` 侧第 `line` 行收起后实际显示的行：被代码折叠隐藏时为隐藏内容之前的一行，
    /// 否则为 `line`。
    pub fn visible_code_line(&self, side: usize, line: usize) -> usize {
        let hidden = &self.code_hidden[side];
        let i = hidden.partition_point(|range| range.end <= line);
        match hidden.get(i) {
            Some(range) if range.start <= line => range.start.checked_sub(1).unwrap_or(range.end),
            _ => line,
        }
    }

    /// 第 `side` 侧第 `line` 行是否被代码折叠隐藏。
    pub(super) fn code_hidden(&self, side: usize, line: usize) -> bool {
        let hidden = &self.code_hidden[side];
        let i = hidden.partition_point(|range| range.end <= line);
        hidden.get(i).is_some_and(|range| range.start <= line)
    }

    /// 展开在第 `side` 侧隐藏了第 `line` 行的代码折叠，返回是否有改动。不重新排列显示行。
    pub(super) fn reveal_code_line(&mut self, side: usize, line: usize) -> bool {
        let before = self.code_folds.len();
        let keep: Vec<bool> = (self.code_folds.iter())
            .map(|fold| !self.hidden_by(fold)[side].contains(&line))
            .collect();
        let mut keep = keep.into_iter();
        self.code_folds.retain(|_| keep.next().unwrap_or(true));
        self.code_folds.len() != before
    }

    /// 第 `side` 侧从第 `first` 行起改动、行数变化 `delta` 后，这一侧的折叠跟着移动。
    ///
    /// 改动落在块内（含首行）的折叠不再保留；块在改动之后的整体移动。按一处连续的修改估算，
    /// 与固定显示的行相同。
    pub(super) fn follow_code_edit(&mut self, side: usize, first: usize, delta: isize) {
        self.code_folds
            .retain(|fold| fold.side != side || !(fold.start..=fold.end).contains(&first));
        for fold in self.code_folds.iter_mut().filter(|f| f.side == side) {
            if fold.start > first {
                fold.start = fold.start.saturating_add_signed(delta).max(first);
                fold.end = fold.end.saturating_add_signed(delta).max(fold.start);
            }
        }
    }

    /// 采用刷新前的差异 `old` 中、文字未变的一侧的代码折叠。
    pub(super) fn keep_code_folds(&mut self, old: &DiffView) {
        self.code_folds = (old.code_folds.iter())
            .filter(|fold| self.side(fold.side).text == old.side(fold.side).text)
            .copied()
            .collect();
    }

    /// 计算两侧被代码折叠隐藏的行，按开头排序并合并相交的部分；排列显示行之前调用。
    pub(super) fn update_code_hidden(&mut self) {
        let mut hidden = [Vec::new(), Vec::new()];
        for fold in &self.code_folds {
            for (side, range) in self.hidden_by(fold).into_iter().enumerate() {
                if !range.is_empty() {
                    hidden[side].push(range);
                }
            }
        }
        for ranges in &mut hidden {
            ranges.sort_by_key(|range| range.start);
            let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
            for range in ranges.drain(..) {
                match merged.last_mut() {
                    Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                    _ => merged.push(range),
                }
            }
            *ranges = merged;
        }
        self.code_hidden = hidden;
    }

    /// 折叠 `fold` 在两侧各隐藏的行：块所在一侧是首行之后到末行，另一侧是对应的行。
    ///
    /// 未改动的内容两侧逐行对应，隐藏的行数相同，两侧保持对齐；改动中的行在另一侧没有对应，
    /// 隐藏范围的两端落在改动中间时，另一侧保留整段改动。
    fn hidden_by(&self, fold: &CodeFold) -> [Range<usize>; 2] {
        let own = fold.start + 1..fold.end + 1;
        let other = self.map_boundary(fold.side, own.start, Edge::Start)
            ..self.map_boundary(fold.side, own.end, Edge::End);
        let other = other.start..other.end.max(other.start);
        if fold.side == LEFT {
            [own, other]
        } else {
            [other, own]
        }
    }

    /// 把第 `side` 侧的行边界 `b`（第 `b` 行之前）映射到另一侧。
    fn map_boundary(&self, side: usize, b: usize, edge: Edge) -> usize {
        let lines = |h: &winc_core::diff::LineHunk| {
            if side == LEFT {
                (h.old.clone(), h.new.clone())
            } else {
                (h.new.clone(), h.old.clone())
            }
        };
        // 第一段结束晚于 `b` 的改动：`b` 在这段改动中间，或在其前面的未改动内容中。
        let i = self.hunks.partition_point(|h| lines(h).0.end <= b);
        if let Some(h) = self.hunks.get(i)
            && lines(h).0.start < b
            && b < lines(h).0.end
        {
            let other = lines(h).1;
            return match edge {
                Edge::Start => other.end,
                Edge::End => other.start,
            };
        }
        // 未改动的内容中：与上一段改动的结尾相距的行数两侧相同。
        let (own, other) = i.checked_sub(1).map_or((0, 0), |p| {
            (lines(&self.hunks[p]).0.end, lines(&self.hunks[p]).1.end)
        });
        other + (b - own)
    }

    /// 返回右侧显示行 `row` 显示的行，用来在修改折叠后找回同一处内容；没有该行时为最大值。
    fn anchor_line(&self, row: usize) -> usize {
        match self.rows[RIGHT].get(row) {
            Some(SideRow::Line { line, .. }) => *line,
            Some(SideRow::Fold { lines, .. }) => lines.start,
            None => usize::MAX,
        }
    }

    /// 折叠修改之后重新排列，返回值同 [`Self::run_code_fold`]。
    fn finish_code_fold(
        &mut self,
        before: Vec<CodeFold>,
        anchor: usize,
        side: usize,
        line: usize,
    ) -> (usize, bool, usize) {
        self.code_folds.sort_by_key(|fold| (fold.side, fold.start));
        let changed = self.code_folds != before;
        if changed {
            self.layout();
        }
        let visible = self.visible_code_line(side, line);
        let anchor = self.visible_code_line(RIGHT, anchor);
        let row = (self.row_of_new(anchor)).min(self.row_count().saturating_sub(1));
        (row, changed, visible)
    }
}

/// 第 `side` 侧以 `start` 开头的块对应的折叠；`ranges` 中没有时末行取首行。
fn fold_of(side: usize, start: usize, ranges: &[FoldRange]) -> CodeFold {
    let end = ranges
        .iter()
        .find(|r| r.start == start)
        .map_or(start, |r| r.end);
    CodeFold { side, start, end }
}

#[cfg(test)]
mod tests;
