//! 折叠代码：折叠的块只剩开头那一行，其余行不显示。
//!
//! 块按缩进划分，Markdown 按标题划分（见 [`scopes::fold_ranges`]）。折叠的块记录开头那一行和最后一
//! 行的行尾（字符偏移），文本变化时随之移动（[`Tracked`]）；修改在块内部时保持折叠，跨过块的边界时
//! 展开该块，避免修改用户看不到的文本。
//!
//! 编辑区按显示行排列：折叠的行不占显示行，自动换行时一行可以占多个显示行（见 [`Wrap`]）。

mod commands;
mod wrapping;

use wrapping::*;

pub(crate) use wrapping::line_breaks;

use std::ops::{ControlFlow, Range};
use std::rc::Rc;

use winc_core::text::{EditBuffer, MAX_LINE_CHARS};
use winc_core::wrap::{self, Breaks};
use winc_core::{Lines, Selection};

use crate::document::Document;
use crate::document::content::Content;
use crate::editing::scopes::{self, FoldRange};
use crate::navigation::bookmarks::Bookmarks;

/// 折叠的一块：`start` 行保留显示，`start + 1..=end` 行不显示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    /// 折叠块的第一行（从 0 起），始终显示。
    pub start: usize,
    /// 折叠块的最后一行（从 0 起），不显示。
    pub end: usize,
    /// `start` 行的行尾位置（字符偏移）；文本变化时随之移动。
    head: usize,
    /// `end` 行的行尾位置（字符偏移）；文本变化时随之移动。
    tail: usize,
}

impl Fold {
    fn new(text: &impl Lines, start: usize, end: usize) -> Self {
        Fold {
            start,
            end,
            head: text.line_end(start),
            tail: text.line_end(end),
        }
    }
}

/// 一个文档中折叠的块，以及由它们算出的不显示的行。
#[derive(Debug, Clone)]
pub struct Folds {
    /// 按开头行排序；块可以嵌套（外层展开后内层仍保持折叠）。
    list: Vec<Fold>,
    /// 「只显示匹配的行」隐藏的行：若干段连续的行（含两端，行号从 0 起），按顺序排列，互不相交。
    ///
    /// 与折叠的块不同，可以从第 0 行开始，前面没有保留显示的那一行。
    filter: Vec<(usize, usize)>,
    /// 不显示的行：若干段连续的行（含两端，行号从 0 起），按顺序排列，互不相交。
    hidden: Vec<(usize, usize)>,
    /// `before[i]` 是 `hidden[..=i]` 共包含的行数（个）。
    ///
    /// 块数很多时（例如大文件全部折叠后），行号与显示行的互换仍然只需二分查找。
    before: Vec<usize>,
    /// 自动换行的设置与各行的段数；`None` 表示没有开启。
    wrap: Option<Wrap>,
    /// 显示着的、被断成多段的行，按行号排序，用于换算显示行。
    wrapped: Vec<Wrapped>,
    /// 文本修改后 `Wrap::extra` 是否有变化、`wrapped` 尚未随之重算。
    ///
    /// 多数按键只改动一行里的字，那一行断成的段数不变，`wrapped` 仍然可用。自动换行的大文件里被
    /// 断开的行可达数十万行，每次按键都重新计算会占去输入的大部分时间。
    wrapped_stale: bool,
}

/// 自动换行的设置：断行宽度，以及被断成多段的行各多占几个显示行。
#[derive(Debug, Clone)]
pub(crate) struct Wrap {
    /// 断行宽度（显示列）。
    pub width: usize,
    /// 一个 Tab 的显示宽度（显示列）。
    pub tab_width: usize,
    /// 被断开的行及各自多占的显示行数：（行号，多占的显示行数），按行号排序。
    extra: Vec<(usize, usize)>,
    /// 计算时对应的文档版本；与文档当前版本对不上时整个重算（例如改名后 CSV 的列对齐可能
    /// 随之变化）。
    ///
    /// 文本修改、末尾追加时只重算改到的行，并同步记录新的版本。
    pub revision: u64,
}

impl Wrap {
    /// 第 `line` 行是否被断成了多段（二分查找，不读文字）。
    pub fn breaks_line(&self, line: usize) -> bool {
        self.extra.binary_search_by_key(&line, |&(l, _)| l).is_ok()
    }
}

/// 一行显示着且被断成多段。
#[derive(Debug, Clone, Copy)]
struct Wrapped {
    /// 行号（从 0 起）。
    line: usize,
    /// 不计换行时这一行的显示行号（从 0 起，见 [`Folds::visible_index`]）。
    vis: usize,
    /// 这一行之前显示着的行共多占的显示行数（显示行）。
    before: usize,
    /// 这一行自己多占的显示行数（显示行）。
    extra: usize,
}

impl Default for Folds {
    fn default() -> Self {
        Folds {
            list: Vec::new(),
            filter: Vec::new(),
            hidden: Vec::new(),
            before: vec![0],
            wrap: None,
            wrapped: Vec::new(),
            wrapped_stale: false,
        }
    }
}

impl Folds {
    /// 是否没有折叠的块、也没有在筛选，即每一行都显示。
    pub fn is_empty(&self) -> bool {
        self.list.is_empty() && self.filter.is_empty()
    }

    /// 是否处于「只显示匹配的行」状态。
    pub fn is_filtered(&self) -> bool {
        !self.filter.is_empty()
    }

    /// 返回按开头行排序的所有折叠块。
    pub fn iter(&self) -> impl Iterator<Item = &Fold> {
        self.list.iter()
    }

    /// 返回 `line` 所在的那一段不显示的行（含两端）；`line` 显示着时返回 `None`。
    fn hidden_span(&self, line: usize) -> Option<(usize, usize)> {
        let i = self.hidden.partition_point(|&(_, b)| b < line);
        self.hidden.get(i).copied().filter(|&(a, _)| a <= line)
    }

    /// 是否不显示第 `line` 行：在折叠块内部或被筛选掉时为 `true`。
    pub fn is_hidden(&self, line: usize) -> bool {
        self.hidden_span(line).is_some()
    }

    /// 是否有以第 `line` 行为开头的折叠块（不管这一行自己是否显示）。
    fn has(&self, line: usize) -> bool {
        self.list.binary_search_by_key(&line, |f| f.start).is_ok()
    }

    /// 是否第 `line` 行是一个折叠块的开头且该行显示着。
    pub fn is_collapsed(&self, line: usize) -> bool {
        !self.is_hidden(line) && self.has(line)
    }

    /// 返回不计换行时第 `line` 行所在的显示行号（在显示着的行里排第几，从 0 起）。
    fn visible_index(&self, line: usize) -> usize {
        let line = match self.hidden_span(line) {
            Some((0, _)) => return 0,
            Some((a, _)) => a - 1,
            None => line,
        };
        line - self.before[self.hidden.partition_point(|&(a, _)| a < line)]
    }

    /// 返回第 `line` 行之前显示着、被断开的行共多占的显示行数。
    fn extra_before_line(&self, line: usize) -> usize {
        let i = self.wrapped.partition_point(|w| w.line < line);
        i.checked_sub(1)
            .map_or(0, |j| self.wrapped[j].before + self.wrapped[j].extra)
    }

    /// 返回第 `line` 行的第一段所在的显示行号（从 0 起）。
    ///
    /// 不显示的行算在它前面最近一行显示着的行上；从第 0 行起就隐藏的行算在第 0 个显示行。
    pub fn row_of(&self, line: usize) -> usize {
        let shown = match self.hidden_span(line) {
            Some((0, _)) => return 0,
            Some((a, _)) => a - 1,
            None => line,
        };
        self.visible_index(shown) + self.extra_before_line(shown)
    }

    /// 返回第 `line` 行（应是显示着的行）占的显示行数。
    pub fn rows_of(&self, line: usize) -> usize {
        let i = self.wrapped.partition_point(|w| w.line < line);
        1 + self
            .wrapped
            .get(i)
            .filter(|w| w.line == line)
            .map_or(0, |w| w.extra)
    }

    /// 返回第 `row` 个显示行对应的（行号，段号），均从 0 起。
    pub fn line_and_part(&self, row: usize) -> (usize, usize) {
        // 最后一处起点不晚于 `row` 的被断开的行。
        let i = self.wrapped.partition_point(|w| w.vis + w.before <= row);
        if let Some(w) = i.checked_sub(1).map(|j| self.wrapped[j]) {
            let start = w.vis + w.before;
            if row <= start + w.extra {
                return (w.line, row - start);
            }
            return (self.line_of_visible(row - w.before - w.extra), 0);
        }
        (self.line_of_visible(row), 0)
    }

    /// 返回第 `row` 个显示行对应的行号（从 0 起）。
    pub fn line_of_row(&self, row: usize) -> usize {
        self.line_and_part(row).0
    }

    /// 返回不计换行时第 `row` 个显示行对应的行号（从 0 起）。
    fn line_of_visible(&self, row: usize) -> usize {
        // 第 i 段之后的那一行位于第 `a - before[i]` 个显示行；该行不在 `row` 之后时，要跳过这一段。
        let (mut lo, mut hi) = (0, self.hidden.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.hidden[mid].0 - self.before[mid] <= row {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        row + self.before[lo]
    }

    /// 返回 `line_count` 行的文件共有多少个显示行（至少 1 个）。
    pub fn row_count(&self, line_count: usize) -> usize {
        let hidden = self.before.last().copied().unwrap_or(0);
        let extra = self.wrapped.last().map_or(0, |w| w.before + w.extra);
        line_count.saturating_sub(hidden).max(1) + extra
    }

    /// 是否开着自动换行。
    pub fn is_wrapping(&self) -> bool {
        self.wrap.is_some()
    }

    /// 返回从 `line` 起向下第一行显示着的行的行号（`line` 自己显示着时就是它）。
    pub fn visible_from(&self, line: usize) -> usize {
        self.hidden_span(line).map_or(line, |(_, b)| b + 1)
    }

    /// 返回从第 `first_row` 个显示行起向下 `rows` 个显示行对应的行：若干段连续的行（左闭右开，
    /// 行号从 0 起）；第一行可以只露出后面几段。
    pub fn segments(&self, first_row: usize, rows: usize, line_count: usize) -> Vec<Range<usize>> {
        if self.wrapped.is_empty() {
            return self.visible_segments(first_row, rows, line_count);
        }
        // 换算成不计换行的显示行：从第一行到最后一行（含）。
        let first = self.line_of_row(first_row);
        let last = self.line_of_row(first_row + rows.max(1) - 1);
        let (v0, v1) = (self.visible_index(first), self.visible_index(last));
        self.visible_segments(v0, v1 - v0 + 1, line_count)
    }

    fn visible_segments(
        &self,
        first_row: usize,
        rows: usize,
        line_count: usize,
    ) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut line = self.line_of_visible(first_row);
        let mut left = rows;
        while left > 0 && line < line_count {
            let next = self
                .hidden
                .get(self.hidden.partition_point(|&(a, _)| a <= line))
                .copied();
            let stop = next
                .map_or(line_count, |(a, _)| a)
                .min(line + left)
                .min(line_count);
            out.push(line..stop);
            left -= stop - line;
            line = match next {
                Some((a, b)) if stop == a => b + 1,
                _ => stop,
            };
        }
        out
    }

    /// 返回折叠前后差异所在的显示行范围；差异不是单独一片时返回 `None`。
    ///
    /// `open` 是展开状态，`closed` 是折叠状态；差异指在 `open` 中显示、在 `closed` 中折起的行，
    /// 范围按 `open` 的显示行号（从 0 起）计。只有一处且连成一片时（折起或展开一块，连同它里面的
    /// 块）才有结果，动画作用在这一片上；差异分成几处（例如全部折叠）时返回 `None`。
    pub fn changed_rows(open: &Folds, closed: &Folds) -> Option<Range<usize>> {
        let inside = |(a, b): (usize, usize)| {
            closed
                .hidden_span(a)
                .is_some_and(|(ca, cb)| ca <= a && b <= cb)
        };
        if !open.hidden.iter().all(|&span| inside(span)) {
            return None;
        }

        // `closed` 的每一段减去 `open` 里本来就隐藏的行，剩下的在 `open` 里都显示着。
        let mut found: Option<Range<usize>> = None;
        for &(ca, cb) in &closed.hidden {
            let mut at = ca;
            let first = open.hidden.partition_point(|&(_, b)| b < ca);
            let mut gaps = Vec::new();
            for &(a, b) in open.hidden[first..].iter().take_while(|&&(a, _)| a <= cb) {
                if a > at {
                    gaps.push((at, a - 1));
                }
                at = at.max(b + 1);
            }
            if at <= cb {
                gaps.push((at, cb));
            }
            for (a, b) in gaps {
                let rows = open.row_of(a)..open.row_of(b) + open.rows_of(b);
                found = match found {
                    None => Some(rows),
                    Some(prev) if prev.end == rows.start => Some(prev.start..rows.end),
                    Some(_) => return None,
                };
            }
        }
        found
    }

    /// 折叠 `folds` 中的块，已经折叠的跳过，返回是否有新折叠的块。
    ///
    /// 全部折叠时一次插入的块可达数万个，因此最后才重算一次。
    fn insert(&mut self, folds: impl IntoIterator<Item = Fold>) -> bool {
        let new: Vec<Fold> = folds
            .into_iter()
            .filter(|f| f.end > f.start && !self.has(f.start))
            .collect();
        if new.is_empty() {
            return false;
        }
        self.list.extend(new);
        self.list.sort_by_key(|f| f.start);
        self.list.dedup_by_key(|f| f.start);
        self.rebuild();
        true
    }

    /// 移除满足 `drop` 的块，返回是否有块被移除。
    fn remove(&mut self, drop: impl Fn(&Fold) -> bool) -> bool {
        let before = self.list.len();
        self.list.retain(|f| !drop(f));
        let changed = self.list.len() != before;
        if changed {
            self.rebuild();
        }
        changed
    }

    fn clear(&mut self) {
        self.list.clear();
        self.filter.clear();
        self.rebuild();
    }

    /// 只显示 `shown` 列出的行（按顺序），第 `end` 行起都显示（尚未搜索到那里）；`shown` 为 `None`
    /// 时取消筛选。
    fn set_filter(&mut self, shown: Option<&[usize]>, line_count: usize, end: usize) {
        self.filter.clear();
        if let Some(shown) = shown {
            let end = end.min(line_count);
            let mut next = 0;
            for &line in shown.iter().filter(|&&l| l < end) {
                if line > next {
                    self.filter.push((next, line - 1));
                }
                next = next.max(line + 1);
            }
            if next < end {
                self.filter.push((next, end - 1));
            }
            // 一行都不显示时保留最后一行，使编辑区不至于空白。文件只有一行时无法保留这一行，于是
            // 不筛选，这一行仍然显示。
            if shown.is_empty() && end == line_count && end <= 1 {
                self.filter.clear();
            } else if shown.is_empty() && end == line_count {
                if let Some(last) = self.filter.last_mut() {
                    last.1 -= 1;
                }
                if let Some(&(a, b)) = self.filter.last()
                    && b < a
                {
                    self.filter.pop();
                }
            }
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.hidden.clear();
        let mut spans: Vec<(usize, usize)> = self
            .list
            .iter()
            .map(|fold| (fold.start + 1, fold.end))
            .chain(self.filter.iter().copied())
            .collect();
        if !self.filter.is_empty() {
            spans.sort_unstable();
        }
        for (a, b) in spans {
            match self.hidden.last_mut() {
                // 开头那一行已经在上一段里（嵌套在内的块），或与上一段紧挨：合并进上一段。
                Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
                _ => self.hidden.push((a, b)),
            }
        }
        self.before.clear();
        self.before.push(0);
        let mut total = 0;
        for &(a, b) in &self.hidden {
            total += b - a + 1;
            self.before.push(total);
        }
        self.rebuild_wrapped();
    }

    /// 按当前的折叠和换行重新计算 `wrapped`。
    fn rebuild_wrapped(&mut self) {
        self.wrapped.clear();
        self.wrapped_stale = false;
        let Some(wrap) = self.wrap.take() else {
            return;
        };
        let mut before = 0;
        for &(line, extra) in &wrap.extra {
            if self.is_hidden(line) {
                continue;
            }
            let vis = self.visible_index(line);
            self.wrapped.push(Wrapped {
                line,
                vis,
                before,
                extra,
            });
            before += extra;
        }
        self.wrap = Some(wrap);
    }

    /// 开启、关闭自动换行或更换宽度：按 `text` 整个重新计算。
    ///
    /// `wrap` 是（断行宽度，Tab 宽度），单位均为显示列，`None` 表示关闭；`revision` 是此时的文档
    /// 版本号。
    pub(crate) fn set_wrap(
        &mut self,
        text: &impl Lines,
        wrap: Option<(usize, usize)>,
        revision: u64,
    ) {
        self.wrap = wrap.map(|(width, tab_width)| {
            // 按顺序遍历各行：逐行按行号定位的开销与行数成正比，10 万行约 20 ms。每行最多取
            // `MAX_LINE_CHARS` 个字符所需的字节，与 `line_breaks` 看的字符相同。
            let mut extra = Vec::new();
            text.visit_lines(0, MAX_LINE_CHARS * 4, |line, _, content| {
                // 每个字符占的显示列数不超过它的 UTF-8 字节数（Tab 除外），按字节数放得下的行
                // 不必逐字符测量。
                let tabs = content.bytes().filter(|&b| b == b'\t').count();
                let cells = content.len() + tabs * tab_width.saturating_sub(1);
                if cells <= width {
                    return ControlFlow::Continue(());
                }
                let rows = breaks_of(content.len(), || content.chars(), width, tab_width).rows();
                if rows > 1 {
                    extra.push((line, rows - 1));
                }
                ControlFlow::Continue(())
            });
            Wrap {
                width,
                tab_width,
                extra,
                revision,
            }
        });
        self.rebuild();
    }

    /// 返回自动换行的设置；没有开启时返回 `None`。
    pub(crate) fn wrap(&self) -> Option<&Wrap> {
        self.wrap.as_ref()
    }

    /// 返回自动换行设置的可变引用；没有开启时返回 `None`。
    pub(crate) fn wrap_mut(&mut self) -> Option<&mut Wrap> {
        self.wrap.as_mut()
    }

    /// 按改动重新计算断行：改动前是第 `first..=old_last` 行，改动后是第 `first..=new_last` 行。
    fn edit_wrap(&mut self, text: &impl Lines, first: usize, old_last: usize, new_last: usize) {
        let Some(wrap) = &mut self.wrap else {
            return;
        };
        let delta = new_last as isize - old_last as isize;
        let from = wrap.extra.partition_point(|&(l, _)| l < first);
        let to = wrap.extra.partition_point(|&(l, _)| l <= old_last);
        let fresh: Vec<(usize, usize)> = (first
            ..=new_last.min(text.line_count().saturating_sub(1)))
            .filter_map(|line| {
                let rows = line_rows(text, line, wrap.width, wrap.tab_width);
                (rows > 1).then_some((line, rows - 1))
            })
            .collect();
        // 行数没变（行内输入、同一行内的全部替换）时后面的行号不需要移动：全部替换一次可能
        // 修改上万处，每处都遍历后面所有被断开的行会明显变慢。
        if delta != 0 && to < wrap.extra.len() {
            for entry in &mut wrap.extra[to..] {
                entry.0 = entry.0.saturating_add_signed(delta);
            }
            self.wrapped_stale = true;
        }
        if wrap.extra[from..to] != fresh[..] {
            wrap.extra.splice(from..to, fresh);
            self.wrapped_stale = true;
        }
    }

    /// 按一处修改调整折叠块的位置和行号，并移除被修改跨过边界的块。
    ///
    /// 修改是在 `start`（字符偏移）处删除 `removed` 个字符并插入 `inserted`，行数增加 `lines` 行
    /// （减少时为负）。不按位置重新查行号：全部折叠后块数可达数万个，输入一个字符不能遍历全部块。
    fn edit(&mut self, start: usize, removed: usize, inserted: &str, lines: isize) {
        // 文本修改后，筛选出的行号不再对应，等重新搜索完成后再按新的结果筛选。
        self.filter.clear();
        let end = start + removed;
        let delta = inserted.chars().count() as isize - removed as isize;
        let shift = |pos: usize| pos.saturating_add_signed(delta);
        let breaks = inserted.contains(['\n', '\r']);
        self.list.retain_mut(|f| {
            if end < f.head || (end == f.head && !breaks) {
                // 修改在块的开头那一行或更靠上的位置。
                f.head = shift(f.head);
                f.tail = shift(f.tail);
                // 修改都在开头那一行的行尾之前，增减的行都在该行之上。
                f.start = f.start.saturating_add_signed(lines);
                f.end = f.end.saturating_add_signed(lines);
                true
            } else if start > f.head && end <= f.tail {
                // 修改在折叠的行内部。
                f.tail = shift(f.tail);
                f.end = f.end.saturating_add_signed(lines);
                f.end > f.start
            } else {
                start > f.tail
            }
        });
    }

    /// 文本修改后重新计算不显示的行和 `wrapped`。
    ///
    /// 折叠的块可能在这次修改中全部被移除（跨过边界时移除），修改也会取消筛选，因此修改前还有
    /// 不显示的行时就必须整个重算：否则 `hidden` 仍是旧的几段，行会被无故隐藏。修改前后都没有
    /// 不显示的行时 `hidden` 不变，`wrapped` 只在被断开的行有变化时才重算。
    fn refresh(&mut self) {
        if self.is_empty() && self.hidden.is_empty() {
            if self.wrapped_stale {
                self.rebuild_wrapped();
            }
        } else {
            self.rebuild();
        }
    }
}

/// 修改文本时一并移动折叠块和书签的位置。
pub(crate) struct Tracked<'a> {
    /// 被修改的文本。
    pub text: &'a mut Content,
    /// 需要随修改移动的折叠状态。
    pub folds: &'a mut Folds,
    /// 需要随修改移动的书签。
    pub bookmarks: &'a mut Bookmarks,
}

impl EditBuffer for Tracked<'_> {
    fn replace(&mut self, start: usize, removed: usize, inserted: &str) {
        let before = self.text.line_count();
        // 书签需要知道修改落在哪几行上，必须在修改之前计算。
        let (start_line, end_line, whole_lines) = if self.bookmarks.is_empty() {
            (0, 0, false)
        } else {
            let end = (start + removed).min(self.text.end_pos());
            (
                self.text.line_of(start),
                self.text.line_of(end),
                removed > 0
                    && self.text.line_start(self.text.line_of(start)) == start
                    && self.text.line_start(self.text.line_of(end)) == end,
            )
        };
        // 自动换行需要重新计算改动的行。
        let wrap_lines = self.folds.wrap.is_some().then(|| {
            let end = (start + removed).min(self.text.end_pos());
            (self.text.line_of(start), self.text.line_of(end))
        });
        self.text.replace(start, removed, inserted);
        let after = self.text.line_count();
        if !self.folds.is_empty() {
            self.folds
                .edit(start, removed, inserted, after as isize - before as isize);
        }
        if let Some((first, old_last)) = wrap_lines {
            let new_last = old_last.saturating_add_signed(after as isize - before as isize);
            self.folds.edit_wrap(&*self.text, first, old_last, new_last);
        }
        if !self.bookmarks.is_empty() {
            self.bookmarks.edit(
                start_line,
                end_line,
                after as isize - before as isize,
                whole_lines,
            );
        }
    }

    /// 没有要跟着移动的折叠、自动换行和书签时整批交给文本（大文件的片段表一次做完）；否则逐处
    /// 修改，每处都先算好它落在哪几行。
    fn replace_each(&mut self, changes: &[(usize, usize, &str)]) {
        if self.folds.is_empty() && self.folds.wrap.is_none() && self.bookmarks.is_empty() {
            self.text.replace_each(changes);
            return;
        }
        for &(start, removed, inserted) in changes {
            self.replace(start, removed, inserted);
        }
    }
}

/// 折叠的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldCommand {
    /// 折叠光标所在的、仍然展开的最内层块（⌥⌘[）。
    Fold,
    /// 展开光标所在的、已折叠的最内层块（⌥⌘]）。
    Unfold,
    /// 切换光标所在的最内层块：已折叠则展开，已展开则折叠（⌘K ⌘L）。
    Toggle,
    /// 折叠光标所在的块及其内部的所有块（⌘K ⌘[）。
    FoldRecursively,
    /// 展开光标所在的块及其内部的所有块（⌘K ⌘]）。
    UnfoldRecursively,
    /// 折叠所有块（⌘K ⌘0）。
    FoldAll,
    /// 展开所有块（⌘K ⌘J）。
    UnfoldAll,
    /// 折叠第 `n` 层（从 1 起）的所有块，光标所在的块除外（⌘K ⌘1 到 ⌘K ⌘7）。
    FoldLevel(usize),
}

impl Document {}

#[cfg(test)]
mod tests;
