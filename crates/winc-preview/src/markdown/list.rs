//! 顶层列表的原文拆分：按顶层项切出字节范围，并为有序列表重编号。
//!
//! 解析结果中的 `Block::List` 有各列表项的结构，但不保留每项在原文中的范围；拖动列表项
//! 需要按原文重排，因此由本模块的扫描器只处理顶层列表。扫描器与 Markdown 规则位于同一个
//! crate，缩进、标记和围栏代码块的判断不分散到界面层。

use std::ops::Range;

/// 顶层列表的项范围（相对顶层块，字节）和有序列表的编号信息。
pub struct ListItems {
    /// 各顶层项的字节范围。
    pub ranges: Vec<Range<usize>>,
    /// 有序列表的编号信息；无序列表为 `None`。
    pub ordered: Option<OrderedList>,
}

/// 有序列表的编号：起始编号、分隔符，以及各项编号是否相同。
pub struct OrderedList {
    /// 第一项的编号，解析结果中的列表起始值。
    start: u64,
    /// `.` 或 `)`。
    delimiter: u8,
    /// 各项的编号是否全部相同（例如均写作 `1.`）：这种写法由显示时自动递增，重排后保持原样。
    uniform: bool,
}

/// 制表位宽度（列），依据 `CommonMark` 规范 §2.2：制表符按 4 列的制表位展开。
const TAB_STOP: usize = 4;

impl OrderedList {
    /// 第一项的编号。
    pub fn start(&self) -> u64 {
        self.start
    }

    /// 把一项正文第一行的有序标记换成从 [`Self::start`] 起的第 `index` 个编号。
    ///
    /// 内容的起始列变化时，项内后续行同样右移或左移相应的列数，使续行、嵌套列表和围栏代码块
    /// 仍属于这一项；标记后是制表符时起始列按制表位计算。各项编号相同时返回原文。分隔符
    /// 不一致时返回 `None`。
    pub fn renumber(&self, item: &str, index: usize) -> Option<String> {
        let (first, rest) = match item.find('\n') {
            Some(at) => (&item[..at], &item[at..]),
            None => (item, ""),
        };
        let indent = first.len() - first.trim_start_matches([' ', '\t']).len();
        let marker = &first[indent..];
        let digits = marker.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 || marker.as_bytes().get(digits) != Some(&self.delimiter) {
            return None;
        }
        if self.uniform {
            return Some(item.to_owned());
        }
        let number = self.start.checked_add(index as u64)?.to_string();
        let after = &marker[digits + 1..];
        let old = content_column(&first[..indent], digits + 1, after);
        let new = content_column(&first[..indent], number.len() + 1, after);
        let delta = new as isize - old as isize;
        let mut out = String::with_capacity(item.len() + delta.max(0) as usize * 4);
        out.push_str(&first[..indent]);
        out.push_str(&number);
        out.push(self.delimiter as char);
        out.push_str(after);
        for line in rest.split_inclusive('\n') {
            let body = line.trim_end_matches(['\n', '\r']);
            if body.trim().is_empty() {
                out.push_str(line);
                continue;
            }
            if delta > 0 {
                out.extend(std::iter::repeat_n(' ', delta as usize));
                out.push_str(line);
            } else if delta < 0 {
                let spaces = body.len() - body.trim_start_matches(' ').len();
                out.push_str(&line[spaces.min(delta.unsigned_abs())..]);
            } else {
                out.push_str(line);
            }
        }
        Some(out)
    }
}

/// 返回列表项内容的起始列（从 0 起，制表符按 [`TAB_STOP`] 展开）。
///
/// `indent` 是标记前的缩进，`marker` 是标记的字节数，`after` 是标记之后的文字。依据
/// `CommonMark` 规范 §5.2：标记后的空白不足 5 列时内容从空白之后开始，否则（或该行没有内容时）
/// 从标记后 1 列开始。
fn content_column(indent: &str, marker: usize, after: &str) -> usize {
    let advance = |column: usize, byte: u8| match byte {
        b'\t' => column + TAB_STOP - column % TAB_STOP,
        _ => column + 1,
    };
    let start = indent.bytes().fold(0, advance) + marker;
    let after = after.trim_end_matches('\r');
    let space = after.len() - after.trim_start_matches([' ', '\t']).len();
    let column = after.bytes().take(space).fold(start, advance);
    if space == after.len() || column - start >= 5 {
        start + 1
    } else {
        column
    }
}

/// 把顶层列表的原文切成各项的范围；至少两项，否则返回 `None`。
///
/// 同级项指缩进与第一项相同、标记类型也相同的行；缩进更深的行属于当前项。围栏代码块中的
/// 行不切分。
pub fn split_list_items(text: &str) -> Option<ListItems> {
    let mut base = None;
    let mut kind = None;
    let mut item_start = None;
    let mut items = Vec::new();
    let mut offset = 0;
    let mut fence: Option<(u8, usize)> = None;
    let mut uniform = true;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let trimmed = body.trim_start_matches([' ', '\t']);
        let indent = body.len() - trimmed.len();
        if base.is_none() {
            if trimmed.is_empty() {
                offset += line.len();
                continue;
            }
            kind = marker(trimmed);
            kind?;
            base = Some(indent);
        }
        // 围栏代码块的开始与结束：里面的行即使看起来像列表项也不切分。开始行可以带语言，
        // 结束行只能是同样或更多的围栏字符加空白。
        let Some(mark) = trimmed.as_bytes().first().copied() else {
            offset += line.len();
            continue;
        };
        if matches!(mark, b'`' | b'~') {
            let count = trimmed.bytes().take_while(|&b| b == mark).count();
            let closed = match fence {
                Some((open, open_count)) => {
                    open == mark && count >= open_count && trimmed[count..].trim().is_empty()
                }
                None => false,
            };
            if closed {
                fence = None;
            } else if fence.is_none() && count >= 3 {
                fence = Some((mark, count));
            }
            if closed || (fence.is_some() && count >= 3) {
                offset += line.len();
                continue;
            }
        }
        if fence.is_none()
            && Some(indent) == base
            && let Some(found) = marker(trimmed)
            && same_marker(kind.as_ref()?, &found)
        {
            uniform &= Some(found) == kind;
            if let Some(from) = item_start.replace(offset) {
                items.push(from..offset);
            }
        }
        offset += line.len();
    }
    if let Some(from) = item_start.replace(text.len()) {
        items.push(from..text.len());
    }
    if items.len() < 2 {
        return None;
    }
    let ordered = match kind? {
        Marker::Bullet => None,
        Marker::Number {
            value: start,
            delimiter,
            ..
        } => Some(OrderedList {
            start,
            delimiter,
            uniform,
        }),
    };
    Some(ListItems {
        ranges: items,
        ordered,
    })
}

/// 列表项标记。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    /// `-`、`*`、`+`。
    Bullet,
    /// `1.`、`2)` 这类有序标记。
    Number { value: u64, delimiter: u8 },
}

/// 解析一行的列表标记；不是标记时返回 `None`。
fn marker(s: &str) -> Option<Marker> {
    let bytes = s.as_bytes();
    match bytes.first()? {
        b'-' | b'*' | b'+' if marker_end(bytes.get(1)) => Some(Marker::Bullet),
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 {
                return None;
            }
            let delimiter = *bytes.get(digits)?;
            if !matches!(delimiter, b'.' | b')') || !marker_end(bytes.get(digits + 1)) {
                return None;
            }
            let value = s.get(..digits)?.parse().ok()?;
            Some(Marker::Number { value, delimiter })
        }
        _ => None,
    }
}

/// 标记后面是否只隔着空白（或没有字符）。
fn marker_end(next: Option<&u8>) -> bool {
    matches!(next, None | Some(b' ' | b'\t'))
}

/// 两个标记是否属于同一种列表：无序对无序，有序还要分隔符相同。
fn same_marker(first: &Marker, found: &Marker) -> bool {
    match (first, found) {
        (Marker::Bullet, Marker::Bullet) => true,
        (
            Marker::Number {
                delimiter: first, ..
            },
            Marker::Number {
                delimiter: found, ..
            },
        ) => first == found,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
