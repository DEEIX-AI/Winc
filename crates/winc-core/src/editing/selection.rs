//! 选区：锚点和光标两端的位置，以及区间的端点与夹取。

use std::ops::Range;

/// 选区：`anchor` 是起点，`head` 是光标所在的一端。
///
/// 两者相等时选区折叠为一个光标。`anchor` 可以大于 `head`（反向选择）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// 选区起点（位置，从 0 起的 `char` 偏移）。
    pub anchor: usize,
    /// 光标所在的一端（位置，从 0 起的 `char` 偏移）。
    pub head: usize,
}

impl Selection {
    /// 创建一个位于 `pos` 的折叠光标。
    pub fn point(pos: usize) -> Self {
        Self {
            anchor: pos,
            head: pos,
        }
    }

    /// 用起点 `anchor` 和光标端 `head` 创建一个选区。
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// 返回选区的起点，即 `anchor` 与 `head` 中较小的位置（`char` 偏移）。
    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// 返回选区的终点，即 `anchor` 与 `head` 中较大的位置（`char` 偏移）。
    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// 返回选区覆盖的位置范围，左闭右开。
    pub fn range(&self) -> Range<usize> {
        self.from()..self.to()
    }

    /// 是否是折叠光标，即起点与光标端重合、没有选中文本。
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// 返回把光标端移到 `pos` 后的选区。
    ///
    /// `extend` 为真时保留起点以扩展选区，否则返回 `pos` 处的折叠光标。
    pub fn moved_to(&self, pos: usize, extend: bool) -> Self {
        if extend {
            Self {
                anchor: self.anchor,
                head: pos,
            }
        } else {
            Self::point(pos)
        }
    }

    /// 返回两端位置都不超过 `len` 的选区，`len` 是文本的 `char` 数。
    pub fn clamped(&self, len: usize) -> Self {
        Self {
            anchor: self.anchor.min(len),
            head: self.head.min(len),
        }
    }
}

/// 返回整理后的多选区（多光标）和主选区在其中的新下标。
///
/// `primary` 是主选区在 `selections` 中的下标。结果按位置排序：两个选区有共同的部分、或在同一
/// 位置相接且其中一个是折叠光标时合并，合并后的方向沿用主选区，不含主选区时沿用靠前的一个；
/// 两个非空选区首尾相接时各自保留。
///
/// `selections` 为空时返回空表和新下标 0。
pub fn merge(selections: Vec<Selection>, primary: usize) -> (Vec<Selection>, usize) {
    let mut order: Vec<(Selection, bool)> = selections
        .into_iter()
        .enumerate()
        .map(|(i, s)| (s, i == primary))
        .collect();
    order.sort_by_key(|(s, _)| (s.from(), s.to()));

    let mut merged: Vec<(Selection, bool)> = Vec::with_capacity(order.len());
    for (next, is_primary) in order {
        let Some((last, last_primary)) = merged.last_mut() else {
            merged.push((next, is_primary));
            continue;
        };
        let touches = next.from() == last.to() && (last.is_empty() || next.is_empty());
        if next.from() < last.to() || touches {
            let (from, to) = (last.from(), last.to().max(next.to()));
            let backward = if is_primary && !*last_primary {
                next.head < next.anchor
            } else {
                last.head < last.anchor
            };
            *last = if backward {
                Selection::new(to, from)
            } else {
                Selection::new(from, to)
            };
            *last_primary |= is_primary;
        } else {
            merged.push((next, is_primary));
        }
    }

    let primary = merged.iter().position(|&(_, p)| p).unwrap_or(0);
    (merged.into_iter().map(|(s, _)| s).collect(), primary)
}

#[cfg(test)]
mod tests;
