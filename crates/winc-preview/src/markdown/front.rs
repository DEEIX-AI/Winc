//! 文件开头的 YAML front matter（`---` 之间的「键: 值」）：按 GFM 显示成一张表，表头是各个键，
//! 第二行是各个值。

use super::{Align, Block, Inline, Style};

/// 把每行都是「键: 值」的 front matter 排成一张两行表格；有不支持的行时返回 `None`。
///
/// 键读成表头，值读成第二行。值两侧成对的引号去掉；以 `#` 开头的行和空行跳过。
pub(super) fn table(text: &str) -> Option<Block> {
    let mut keys = Vec::new();
    let mut values = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t', '-']) {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        keys.push(cell(key));
        values.push(cell(value));
    }
    if keys.is_empty() {
        return None;
    }
    Some(Block::Table(
        vec![Align::None; keys.len()],
        vec![keys, values],
        true,
    ))
}

fn cell(text: &str) -> Inline {
    let mut inline = Inline::default();
    inline.push(text, Style::default());
    inline
}
