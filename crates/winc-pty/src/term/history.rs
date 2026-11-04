//! 回滚缓冲：滚出屏幕顶部的行，按协议中的 [`Line`] 存放，只记录文字和样式段。

use std::collections::VecDeque;

use super::row::Row;
use crate::protocol::{Line, Marks};

/// 回滚缓冲。
///
/// 每行有一个绝对行号，从终端创建起持续递增，移除旧行不影响其余行的行号。
#[derive(Debug)]
pub(super) struct History {
    /// 从旧到新的行。
    lines: VecDeque<Line>,
    /// 最多保留的行数。
    limit: usize,
    /// `lines[0]` 的绝对行号。
    start: u64,
}

impl History {
    /// 创建最多保留 `limit` 行的回滚缓冲，行号从 0 起。
    pub fn new(limit: usize) -> Self {
        History {
            lines: VecDeque::new(),
            limit,
            start: 0,
        }
    }

    /// 返回现有行的绝对行号范围，右端不含。
    pub fn range(&self) -> (u64, u64) {
        (self.start, self.start + self.lines.len() as u64)
    }

    /// 在末尾追加一行，超过上限时丢弃最旧的行。
    pub fn push(&mut self, line: Line) {
        if self.limit == 0 {
            return;
        }
        self.lines.push_back(line);
        self.trim();
    }

    /// 把滚出屏幕的一行追加到末尾。
    ///
    /// 达到上限时复用最旧一行的内存。
    pub fn push_row(&mut self, row: &Row) {
        if self.limit == 0 {
            return;
        }
        let mut line = if self.lines.len() >= self.limit {
            self.start += 1;
            self.lines.pop_front().unwrap_or_default()
        } else {
            Line::default()
        };
        row.fill_line(&mut line);
        self.lines.push_back(line);
    }

    /// 移除并返回最后一行（屏幕变高时拉回屏幕）。
    pub fn pop(&mut self) -> Option<Line> {
        self.lines.pop_back()
    }

    fn trim(&mut self) {
        while self.lines.len() > self.limit {
            self.lines.pop_front();
            self.start += 1;
        }
    }

    /// 绝对行号 `start..end` 里还在的行：（实际第一行的行号，行）。
    pub fn get(&self, start: u64, end: u64) -> (u64, Vec<Line>) {
        let (first, last) = self.range();
        let start = start.clamp(first, last);
        let end = end.clamp(start, last);
        let lines = self
            .lines
            .range((start - first) as usize..(end - first) as usize)
            .cloned()
            .collect();
        (start, lines)
    }

    /// 按从旧到新的顺序返回带 shell 集成标记的行及其绝对行号。
    pub fn marks(&self) -> impl Iterator<Item = (u64, Marks)> + '_ {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.marks.is_empty())
            .map(|(i, line)| (self.start + i as u64, line.marks))
    }

    /// 取出全部行（供重排使用），再用 [`History::extend`] 放回。
    ///
    /// 行号从原来的起点继续排列。
    pub fn take_all(&mut self) -> VecDeque<Line> {
        std::mem::take(&mut self.lines)
    }

    /// 放入重排后的行，超过上限时丢弃最旧的行（与 [`History::push`] 相同）。
    pub fn extend(&mut self, lines: impl IntoIterator<Item = Line>) {
        for line in lines {
            self.push(line);
        }
    }

    /// 清空回滚缓冲。
    ///
    /// 行号从当前末尾继续排列，主程序用旧行号请求行时返回空结果。
    pub fn clear(&mut self) {
        self.start += self.lines.len() as u64;
        self.lines.clear();
    }
}
