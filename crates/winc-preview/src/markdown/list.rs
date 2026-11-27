//! 顶层列表的原文拆分：按顶层项切出字节范围，并为有序列表重编号。
//!
//! 解析结果中的 `Block::List` 有各列表项的结构，但不再保留每项在原文中的范围；拖动列表项
//! 需要按原文重排，因此这里补一个只处理顶层列表的轻量扫描器。它和 Markdown 规则放在同一个
//! crate，缩进、标记和围栏代码块的判断不会漂到界面层。

use std::ops::Range;

/// 顶层列表的项范围（相对顶层块，字节）和有序列表的编号信息。
pub struct ListItems {
    /// 各顶层项的字节范围。
    pub ranges: Vec<Range<usize>>,
    /// 有序列表的编号信息；无序列表为 `None`。
    pub ordered: Option<OrderedList>,
}

/// 有序列表的编号：起始编号和分隔符。
pub struct OrderedList {
    /// 第一项的编号，解析结果中的列表起始值。
    start: u64,
    /// `.` 或 `)`。
    delimiter: u8,
}

impl OrderedList {
    /// 第一项的编号。
    pub fn start(&self) -> u64 {
        self.start
    }

    /// 把一项正文第一行的有序标记换成从 [`Self::start`] 起的第 `index` 个编号。
    ///
    /// 标记变宽时，项内后续行也右移相同的空格；变窄时去掉多余的缩进。这样续行、嵌套列表和
    /// 围栏代码块仍属于这一项。分隔符不一致时返回 `None`。
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
        let number = self.start.checked_add(index as u64)?;
        let new_digits = number.to_string().len();
        let delta = new_digits as isize - digits as isize;
        let mut out = String::with_capacity(item.len() + delta.max(0) as usize * 4);
        out.push_str(&first[..indent]);
        out.push_str(&number.to_string());
        out.push(self.delimiter as char);
        out.push_str(&marker[digits + 1..]);
        for line in rest.split_inclusive('\n') {
            let eol = if line.ends_with("\r\n") {
                2
            } else if line.ends_with('\n') {
                1
            } else {
                0
            };
            let body = &line[..line.len() - eol];
            if body.trim().is_empty() {
                out.push_str(line);
                continue;
            }
            if delta > 0 {
                out.extend(std::iter::repeat_n(' ', delta as usize));
                out.push_str(line);
            } else if delta < 0 {
                let remove = (-delta) as usize;
                let spaces = body.len() - body.trim_start_matches(' ').len();
                let remove = remove.min(spaces);
                out.push_str(&line[remove..]);
            } else {
                out.push_str(line);
            }
        }
        Some(out)
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
            && let Some(from) = item_start.replace(offset)
        {
            items.push(from..offset);
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
        } => Some(OrderedList { start, delimiter }),
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
mod tests {
    use super::split_list_items;

    #[test]
    fn splits_only_at_sibling_markers() {
        let text = "- 甲\n  - 甲一\n  - 甲二\n- 乙\n";
        let chunks: Vec<&str> = split_list_items(text)
            .unwrap()
            .ranges
            .iter()
            .map(|r| &text[r.clone()])
            .collect();
        assert_eq!(chunks, ["- 甲\n  - 甲一\n  - 甲二\n", "- 乙\n"]);

        let text = "- [ ] 甲\n- [x] 乙\n";
        assert_eq!(split_list_items(text).unwrap().ranges.len(), 2);

        // 围栏代码块里看起来像列表项的行不切分。
        let text = "- 甲\n  ```\n  - 不是项\n  ```\n- 乙\n";
        let chunks: Vec<&str> = split_list_items(text)
            .unwrap()
            .ranges
            .iter()
            .map(|r| &text[r.clone()])
            .collect();
        assert_eq!(chunks, ["- 甲\n  ```\n  - 不是项\n  ```\n", "- 乙\n"]);

        assert!(split_list_items("1. 甲\n").is_none());
        assert!(split_list_items("- 甲\n").is_none());
    }

    #[test]
    fn ordered_lists_renumber_across_marker_widths() {
        let list = split_list_items("5. 甲\n6. 乙\n").unwrap();
        let ordered = list.ordered.as_ref().unwrap();
        assert_eq!(ordered.start(), 5);
        assert_eq!(ordered.renumber("5. 甲", 0).unwrap(), "5. 甲");

        // 第 10 个槽位从一位变两位，续行也要右移一格。
        let last = ordered.renumber("1. 第项\n   续\n", 5).unwrap();
        assert_eq!(last, "10. 第项\n    续\n");

        // 从两位变回一位时去掉续行多出来的缩进。
        let list = split_list_items("1. 甲\n10. 乙\n").unwrap();
        let ordered = list.ordered.as_ref().unwrap();
        assert_eq!(
            ordered.renumber("10. 乙\n    续\n", 0).unwrap(),
            "1. 乙\n   续\n"
        );

        // 分隔符保持 `)`，起始编号保持 10。
        let list = split_list_items("10) 甲\n11) 乙\n12) 丙\n").unwrap();
        let ordered = list.ordered.as_ref().unwrap();
        assert_eq!(
            ordered.renumber("10) 甲\n    续\n", 0).unwrap(),
            "10) 甲\n    续\n"
        );
        assert_eq!(
            ordered.renumber("12) 丙\n    续\n", 0).unwrap(),
            "10) 丙\n    续\n"
        );
    }
}
