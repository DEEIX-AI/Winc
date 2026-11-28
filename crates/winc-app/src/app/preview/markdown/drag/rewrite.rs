//! 拖动放下后改写原文：给出要应用的各处改动，以及旧位置到新位置的映射。
//!
//! 顶层块的移动由移走、插入两处改动组成，改动的大小只与被移动的块有关；列表项的移动改写
//! 列表中顺序变化的部分。移动产生的每条接缝均以空行结束（文末除外），相邻的块不会合成一段。

use std::borrow::Cow;
use std::cmp::Reverse;
use std::ops::Range;

use winc_preview::markdown::ListItems;

/// 改写所读取的原文：整篇文档的 rope，或测试中的字符串。
pub(super) trait Source {
    /// 原文的字节数。
    fn len_bytes(&self) -> usize;
    /// 返回字节范围内的文字；范围两端必须落在字符边界上。
    fn slice(&self, range: Range<usize>) -> Cow<'_, str>;
    /// 从字节偏移 `at` 起向后逐个返回字节。
    fn bytes_from(&self, at: usize) -> impl Iterator<Item = u8> + '_;
    /// 从字节偏移 `at` 之前起向前逐个返回字节。
    fn bytes_before(&self, at: usize) -> impl Iterator<Item = u8> + '_;
}

impl Source for str {
    fn len_bytes(&self) -> usize {
        self.len()
    }

    fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        Cow::Borrowed(&self[range])
    }

    fn bytes_from(&self, at: usize) -> impl Iterator<Item = u8> + '_ {
        self.as_bytes()[at..].iter().copied()
    }

    fn bytes_before(&self, at: usize) -> impl Iterator<Item = u8> + '_ {
        self.as_bytes()[..at].iter().rev().copied()
    }
}

impl Source for ropey::Rope {
    fn len_bytes(&self) -> usize {
        ropey::Rope::len_bytes(self)
    }

    fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        self.byte_slice(range).into()
    }

    fn bytes_from(&self, at: usize) -> impl Iterator<Item = u8> + '_ {
        self.bytes_at(at)
    }

    fn bytes_before(&self, at: usize) -> impl Iterator<Item = u8> + '_ {
        self.bytes_at(at).reversed()
    }
}

/// 一处改动：把旧原文中的字节范围换成新文字。
pub(super) struct Edit {
    /// 旧原文中被替换的字节范围。
    pub(super) range: Range<usize>,
    /// 替换后的文字。
    pub(super) text: String,
}

impl Edit {
    /// 改动前后的字节数之差。
    fn delta(&self) -> isize {
        self.text.len() as isize - self.range.len() as isize
    }
}

/// 一次拖动改写的各处改动和选区映射信息。
pub(super) struct TextMove {
    /// 各处改动，按起点从后往前排列、互不重叠：依次应用时每处的范围仍是旧原文中的位置。
    pub(super) edits: Vec<Edit>,
    /// 随内容移到新位置的各段，靠前的优先。
    pieces: Vec<Piece>,
}

/// 旧原文中搬到新位置的一段，列表项的标记可能被重新编号。
struct Piece {
    /// 段在旧原文中的字节范围；两端均属于这一段，段末的光标同样随段移动。
    old: Range<usize>,
    /// 段内各行在新旧原文中的对应，按旧行首排列，至少一项。
    lines: Vec<LineShift>,
}

/// 一行在新旧原文中的对应：行首若干字节（编号、缩进）被改写，其后的文字相同。
struct LineShift {
    /// 旧原文中的行首（字节偏移）。
    old: usize,
    /// 旧行首被改写的字节数。
    old_head: usize,
    /// 新原文中的行首（字节偏移）。
    new: usize,
    /// 新行首改写后的字节数。
    new_head: usize,
}

impl Piece {
    /// 原样搬动的一段：旧原文的 `old` 移到新原文的 `new` 处。
    fn verbatim(old: Range<usize>, new: usize) -> Piece {
        let lines = vec![LineShift {
            old: old.start,
            old_head: 0,
            new,
            new_head: 0,
        }];
        Piece { old, lines }
    }

    /// 内容可能改写过行首的一段：旧文字 `old` 从 `at` 起，新文字 `new` 从 `to` 起。
    fn rewritten(old: &str, at: usize, new: &str, to: usize) -> Piece {
        let range = at..at + old.len();
        if old == new {
            return Piece::verbatim(range, to);
        }
        let (mut from, mut into) = (at, to);
        let mut lines = Vec::new();
        for (before, after) in old.split_inclusive('\n').zip(new.split_inclusive('\n')) {
            let same = (before.bytes().rev())
                .zip(after.bytes().rev())
                .take_while(|(a, b)| a == b)
                .count();
            lines.push(LineShift {
                old: from,
                old_head: before.len() - same,
                new: into,
                new_head: after.len() - same,
            });
            from += before.len();
            into += after.len();
        }
        if lines.is_empty() {
            return Piece::verbatim(range, to);
        }
        Piece { old: range, lines }
    }

    fn contains(&self, byte: usize) -> bool {
        self.old.start <= byte && byte <= self.old.end
    }

    fn map(&self, byte: usize) -> usize {
        let index = (self.lines.partition_point(|line| line.old <= byte)).saturating_sub(1);
        let Some(line) = self.lines.get(index) else {
            return byte;
        };
        // 被改写的行首中的位置落到新行首的相应处，不超过新行首；其后按相同的文字对应。
        let column = byte.saturating_sub(line.old);
        if column >= line.old_head {
            line.new + line.new_head + (column - line.old_head)
        } else {
            line.new + column.min(line.new_head)
        }
    }
}

impl TextMove {
    /// 把旧原文中的字节位置映射到改写后的原文。
    ///
    /// 被搬动的段（含段末）随段移动；被删除的文字中的位置落到改动的起点；其余位置按前面各处
    /// 改动的长度差平移，插入点上的位置落在插入的文字之后。
    pub(super) fn map(&self, byte: usize) -> usize {
        if let Some(piece) = self.pieces.iter().find(|piece| piece.contains(byte)) {
            return piece.map(byte);
        }
        let inside = (self.edits.iter())
            .find(|edit| edit.range.start < byte && byte < edit.range.end)
            .map_or(byte, |edit| edit.range.start);
        self.shifted(inside)
    }

    /// 按结束于 `byte` 及之前的各处改动平移 `byte`。
    fn shifted(&self, byte: usize) -> usize {
        let delta: isize = (self.edits.iter())
            .filter(|edit| edit.range.end <= byte)
            .map(Edit::delta)
            .sum();
        byte.saturating_add_signed(delta)
    }

    /// 应用全部改动后原文是否不变。
    ///
    /// 逐字节比较到第一处不同为止；只有大段重复的内容才会比较到改动范围的末尾。
    fn unchanged<S: Source + ?Sized>(&self, source: &S) -> bool {
        if self.edits.iter().map(Edit::delta).sum::<isize>() != 0 {
            return false;
        }
        let Some(first) = self.edits.last() else {
            return true;
        };
        let mut old = source.bytes_from(first.range.start);
        let mut cursor = first.range.start;
        for edit in self.edits.iter().rev() {
            let kept = edit.range.start - cursor;
            let same = (source.bytes_from(cursor).take(kept)).eq(old.by_ref().take(kept))
                && edit.text.bytes().eq(old.by_ref().take(edit.text.len()));
            if !same {
                return false;
            }
            cursor = edit.range.end;
        }
        // 长度之差为 0：最后一处改动之后两边对齐，其余文字相同。
        true
    }
}

/// 计算把第 `block` 块移到第 `target` 块之前（等于块数时移到文末）的改动。
///
/// `ranges` 是各顶层块的字节范围，空范围（块间空行生成的空段落、脚注前的分隔线）不算块。
/// 块后到下一块前的空行属于分隔，随块移走，新位置重新给出一个空行；文末是否以换行结束不变。
/// 放在原位、紧邻原位、参数无效或改写后与原文相同时返回 `None`。
pub(super) fn moved_block<S: Source + ?Sized>(
    source: &S,
    ranges: &[Range<usize>],
    block: usize,
    target: usize,
    eol: &str,
) -> Option<TextMove> {
    let len = source.len_bytes();
    let range = ranges.get(block).filter(|range| !range.is_empty())?;
    let start = line_start(source, range.start.min(len));
    // 下一块按原文中的位置找：脚注在解析结果中排在最后，范围可能落在前面的正文之间。
    let end = (ranges.iter())
        .filter(|range| !range.is_empty())
        .map(|range| range.start.min(len))
        .filter(|&at| at > range.start)
        .min()
        .map_or(len, |at| line_start(source, at));
    let destination = match ranges.get(target) {
        Some(range) if !range.is_empty() => line_start(source, range.start.min(len)),
        None if target == ranges.len() => len,
        _ => return None,
    };
    if (start..=end).contains(&destination) {
        return None;
    }
    let chunk = source.slice(start..end);
    let body = &chunk[..body_end(&chunk)];
    if body.is_empty() {
        return None;
    }
    let final_eol = source.bytes_before(len).next() == Some(b'\n');
    let breaks = |at: usize| eol.repeat(missing_breaks(source, at));
    // 移走最后一块时连同其前面的分隔一起去掉，文末不留空行。
    let removal = if end == len {
        Edit {
            range: after_content(source, start, final_eol)..len,
            text: String::new(),
        }
    } else {
        Edit {
            range: start..end,
            text: breaks(start),
        }
    };
    let lead = breaks(destination);
    let tail = if destination < len {
        eol.repeat(2)
    } else if final_eol {
        eol.to_owned()
    } else {
        String::new()
    };
    // 向下移动时移走的一处在插入点之前，插入点随之平移。
    let shift = if destination > end {
        removal.delta()
    } else {
        0
    };
    let moved_to = destination.saturating_add_signed(shift) + lead.len();
    let insertion = Edit {
        range: destination..destination,
        text: format!("{lead}{body}{tail}"),
    };
    let mut edits = vec![removal, insertion];
    edits.sort_by_key(|edit| Reverse(edit.range.start));
    let moved = TextMove {
        edits,
        pieces: vec![Piece::verbatim(start..start + body.len(), moved_to)],
    };
    (!moved.unchanged(source)).then_some(moved)
}

/// 返回把第 `item` 项移到第 `slot` 项之前后的旧项顺序：新顺序第 i 项来自旧的第 `order[i]` 项。
pub(super) fn item_order(item: usize, slot: usize, count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    order.remove(item);
    let insert = if slot > item { slot - 1 } else { slot };
    order.insert(insert, item);
    order
}

/// 把列表的第 `item` 项移到第 `slot` 项之前，返回改动和选区映射。
///
/// `source` 是整个列表块的原文，从文档的字节偏移 `base` 起。每项拆成正文和末尾的分隔，正文
/// 按新顺序排列，分隔留在原来的槽位，紧凑与松散列表的空行、列表之后的空行均不变。有序列表
/// 按新位置重新编号。改动只覆盖新旧文字不同的部分；改写后与原文相同时返回 `None`。
pub(super) fn moved_list_text(
    source: &str,
    base: usize,
    list: &ListItems,
    item: usize,
    slot: usize,
) -> Option<TextMove> {
    let ranges = &list.ranges;
    let count = ranges.len();
    if item >= count || slot > count || slot == item || slot == item + 1 {
        return None;
    }
    let first = ranges.first()?.start;
    let order = item_order(item, slot, count);
    let mut text = String::with_capacity(source.len() - first);
    let mut bodies = Vec::with_capacity(count);
    let mut separators = Vec::with_capacity(count);
    for (index, &old) in order.iter().enumerate() {
        let old_start = ranges.get(old)?.start;
        let old_body = &source[old_start..old_start + body_end(&source[ranges[old].clone()])];
        let body = match &list.ordered {
            Some(ordered) => Cow::Owned(ordered.renumber(old_body, index)?),
            None => Cow::Borrowed(old_body),
        };
        let at = base + first + text.len();
        bodies.push(Piece::rewritten(old_body, base + old_start, &body, at));
        text.push_str(&body);
        // 分隔按槽位保留：第 `index` 个槽位沿用原第 `index` 项末尾的空行。
        let slot_range = ranges[index].clone();
        let sep_start = slot_range.start + body_end(&source[slot_range.clone()]);
        let at = base + first + text.len();
        separators.push(Piece::verbatim(base + sep_start..base + slot_range.end, at));
        text.push_str(&source[sep_start..slot_range.end]);
    }
    let old = &source[first..];
    let (head, tail) = common_ends(old, &text);
    if head == old.len() && head == text.len() {
        return None;
    }
    text.truncate(text.len() - tail);
    text.drain(..head);
    bodies.append(&mut separators);
    Some(TextMove {
        edits: vec![Edit {
            range: base + first + head..base + source.len() - tail,
            text,
        }],
        pieces: bodies,
    })
}

/// 返回两段文字相同的开头和结尾的字节数，均落在字符边界上，两者之和不超过较短的一段。
fn common_ends(old: &str, new: &str) -> (usize, usize) {
    let mut head = (old.bytes())
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(head) {
        head -= 1;
    }
    let limit = old.len().min(new.len()) - head;
    let mut tail = (old.bytes().rev())
        .zip(new.bytes().rev())
        .take(limit)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - tail) {
        tail -= 1;
    }
    (head, tail)
}

/// 返回块或列表项正文的末尾（去掉末尾的空行）。
fn body_end(chunk: &str) -> usize {
    let mut end = chunk.len();
    while end > 0 && matches!(chunk.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    // 最后几行只有空白时同样算作分隔，一并去掉；最后一行末尾的空格是原文的一部分，保留。
    loop {
        let line = chunk[..end].rfind('\n').map_or(0, |at| at + 1);
        if end > line && chunk[line..end].trim().is_empty() {
            end = line;
            while end > 0 && matches!(chunk.as_bytes()[end - 1], b'\n' | b'\r') {
                end -= 1;
            }
        } else {
            break;
        }
    }
    end
}

/// 返回 `at` 所在行的行首：`at` 之前只有缩进时退到行首，否则不变。
///
/// 解析结果中缩进代码块的范围从缩进之后开始；移动时缩进属于块，不留在原处。
fn line_start<S: Source + ?Sized>(source: &S, at: usize) -> usize {
    let mut indent = 0;
    for byte in source.bytes_before(at) {
        match byte {
            b' ' | b'\t' => indent += 1,
            b'\n' => break,
            _ => return at,
        }
    }
    at - indent
}

/// 返回要在行首 `at` 处补的行尾数，使 `at` 之前的内容以空行结束；文首和已有空行时为 0。
fn missing_breaks<S: Source + ?Sized>(source: &S, at: usize) -> usize {
    let mut breaks = 0;
    for byte in source.bytes_before(at) {
        match byte {
            b'\n' => breaks += 1,
            b' ' | b'\t' | b'\r' => {}
            _ => return 2usize.saturating_sub(breaks),
        }
        if breaks >= 2 {
            return 0;
        }
    }
    0
}

/// 返回 `at` 之前最后一行内容的末尾；`with_eol` 为真时包含该行的行尾。
fn after_content<S: Source + ?Sized>(source: &S, at: usize, with_eol: bool) -> usize {
    let blank = (source.bytes_before(at))
        .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .count();
    let content = at - blank;
    let gap: Vec<u8> = source.bytes_from(content).take(blank).collect();
    let spaces = gap
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    let eol = match &gap[spaces..] {
        [b'\r', b'\n', ..] => 2,
        [b'\n', ..] => 1,
        _ => 0,
    };
    content + spaces + if with_eol { eol } else { 0 }
}

#[cfg(test)]
mod tests;
