//! 拖动放下后改写原文：按新顺序拼出替换文字，并给出旧位置到新位置的映射。

use std::ops::Range;

use winc_preview::markdown::ListItems;

/// 一次拖动改写的原文范围和选区映射信息。
pub(super) struct TextMove {
    /// 要替换的绝对字节范围。
    pub(super) span: Range<usize>,
    /// 替换后的文字。
    pub(super) text: String,
    /// 旧文本中未改动、只换了位置的各段，以及它们在新文本中的绝对字节起点。
    pub(super) chunks: Vec<(Range<usize>, usize)>,
}

impl TextMove {
    /// 把旧文本中的字节位置映射到改写后的新文本。被移动、搬走的段落按段内偏移映射到新位置；
    /// 范围之外按长度差平移；落在这两者之间的空档里时退到替换范围的开头。
    pub(super) fn map(&self, byte: usize) -> usize {
        // 两个相邻的段在边界上会同时包含同一个字节：优先把它算作后一段的起点，这样
        // 替换范围开头的光标不会跟着被拖走的段跑到另一边。
        if let Some((_, start)) = self.chunks.iter().find(|(range, _)| byte == range.start) {
            return *start;
        }
        if let Some((range, start)) = self
            .chunks
            .iter()
            .find(|(range, _)| byte > range.start && byte <= range.end)
        {
            return start + (byte - range.start);
        }
        if byte < self.span.start {
            return byte;
        }
        if byte >= self.span.end {
            let delta = self.text.len() as isize - self.span.len() as isize;
            return (byte as isize + delta).max(0) as usize;
        }
        self.span.start
    }
}

/// 计算把第 `block` 块移到放置位置 `target` 后，需要替换的原文范围和新文字。
///
/// `ranges` 是各顶层块在原文中的字节范围，`eol` 是文档的行尾。块末尾原有的空行属于分隔，
/// 移走时由目标位置重新给出：移到另一块之前时保留一个空行，避免与相邻块合成一段；移到文末时
/// 只保留一个行尾。返回 `None` 表示不需要改动（放在原位、紧邻原位或参数无效）。
#[cfg(test)]
pub(super) fn moved_text(
    source: &str,
    ranges: &[Range<usize>],
    block: usize,
    target: usize,
    eol: &str,
) -> Option<TextMove> {
    moved_text_with(source.len(), ranges, block, target, eol, |range| {
        source[range].to_owned()
    })
}

/// [`moved_text`] 的通用形式：只按需要取原文片段，避免为了移动一块而复制整篇文档。
pub(super) fn moved_text_with<F>(
    len: usize,
    ranges: &[Range<usize>],
    block: usize,
    target: usize,
    eol: &str,
    mut slice: F,
) -> Option<TextMove>
where
    F: FnMut(Range<usize>) -> String,
{
    if block >= ranges.len() || target > ranges.len() || target == block || target == block + 1 {
        return None;
    }
    let start = ranges[block].start.min(len);
    // 下一块按原文中的位置找：脚注在解析结果中排在最后，范围可能落在前面的正文之间。
    let end = ranges
        .iter()
        .map(|range| range.start.min(len))
        .filter(|&at| at > start)
        .min()
        .unwrap_or(len);
    if start >= end {
        return None;
    }
    let destination = ranges.get(target).map_or(len, |range| range.start.min(len));
    if destination >= end {
        // 向下移动：中间各块整体上移，被拖的块放到它们之后。中间部分的末尾统一补成
        // 一个空行，否则块与块之间只剩一个换行，会被 Markdown 当成同一段（文末也一样）。
        let mut gap = slice(end..destination);
        let trimmed = gap.trim_end_matches(['\n', '\r']).len();
        gap.truncate(trimmed);
        gap.push_str(eol);
        gap.push_str(eol);
        let moved = separated(&slice(start..end), eol, destination < len);
        let moved_start = start + gap.len();
        let chunks = vec![(start..end, moved_start), (end..destination, start)];
        return Some(TextMove {
            span: start..destination,
            text: format!("{gap}{moved}"),
            chunks,
        });
    }
    if destination < start {
        let gap = slice(destination..start);
        let moved = separated(&slice(start..end), eol, true);
        let gap_start = destination + moved.len();
        let chunks = vec![(start..end, destination), (destination..start, gap_start)];
        return Some(TextMove {
            span: destination..end,
            text: format!("{moved}{gap}"),
            chunks,
        });
    }
    None
}

/// 返回把第 `item` 项移到第 `slot` 项之前后的旧项顺序：新顺序第 i 项来自旧的第 `order[i]` 项。
pub(super) fn item_order(item: usize, slot: usize, count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    order.remove(item);
    let insert = if slot > item { slot - 1 } else { slot };
    order.insert(insert, item);
    order
}

/// 把列表的第 `item` 项移到第 `slot` 项之前，返回整段列表的新文字和选区映射。
///
/// 列表项范围的末尾可能带着列表后的空行；这里把每项拆成正文和末尾的分隔，正文按新顺序
/// 排列，分隔仍留在原来的槽位，因此紧凑/松散列表的空行和列表与后文之间的空行都不变。
/// 有序列表在正文的标记上重新编号，位数变化时同步调整续行缩进。
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
    let order = item_order(item, slot, count);
    let span_start = base + ranges[0].start;
    let mut text = String::with_capacity(source.len());
    let mut chunks = Vec::with_capacity(count * 2);
    for (index, &old) in order.iter().enumerate() {
        let old_range = ranges[old].clone();
        let body_end = item_body_end(&source[old_range.clone()]);
        let body = &source[old_range.start..old_range.start + body_end];
        let body = match &list.ordered {
            Some(ordered) => ordered.renumber(body, index)?,
            None => body.to_owned(),
        };
        let body_start = span_start + text.len();
        text.push_str(&body);
        chunks.push((
            base + old_range.start..base + old_range.start + body_end,
            body_start,
        ));
        // 分隔按槽位保留：第 `index` 个槽位沿用原第 `index` 项末尾的空行。
        let slot_range = ranges[index].clone();
        let slot_body_end = item_body_end(&source[slot_range.clone()]);
        let sep_start = base + slot_range.start + slot_body_end;
        let sep = &source[slot_range.start + slot_body_end..slot_range.end];
        let new_sep = span_start + text.len();
        text.push_str(sep);
        chunks.push((sep_start..base + slot_range.end, new_sep));
    }
    Some(TextMove {
        span: span_start..base + source.len(),
        text,
        chunks,
    })
}

/// 返回块或列表项正文的末尾（去掉末尾的空行）。
fn item_body_end(chunk: &str) -> usize {
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

/// 去掉块末尾的空行，再补上块与下一块之间的空行；移到文末时只补一个行尾。
///
/// 块末尾原有的空行属于分隔，不属于块的内容：移走之后由目标位置重新给出。
fn separated(chunk: &str, eol: &str, blank_line: bool) -> String {
    let end = item_body_end(chunk);
    let mut moved = String::with_capacity(end + 2 * eol.len());
    moved.push_str(&chunk[..end]);
    moved.push_str(eol);
    if blank_line {
        moved.push_str(eol);
    }
    moved
}

#[cfg(test)]
mod tests;
