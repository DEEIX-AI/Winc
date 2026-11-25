//! 位置换算：char 偏移 ↔ LSP Position（行号 + UTF-16 code unit 列）。
//!
//! 编辑器全库用 char 偏移，LSP 默认用 UTF-16 code unit 列（BMP 外的字符算 2 个）。
//! 换算拆成两层：行定位由调用方完成（文档侧的 rope 是 O(log n)），行内 char ↔ UTF-16
//! 在本模块做（O(行长)，行内遍历一次）。因此本 crate 不依赖 ropey：接收行文本与行内
//! 偏移，大文件（`Content::Large`）与小文件走同一条路。
//!
//! 代理对的中间位置（UTF-16 单数落在高低代理之间）向后取整到字符边界，与 LSP 规定的
//! 客户端容错一致：宁可位置稍偏，不可崩溃。

use crate::message::Position;

/// 行内前 `chars` 个字符的 UTF-16 code unit 数。`chars` 超过行长度时按行尾算。
pub fn chars_to_utf16(line: &str, chars: usize) -> u32 {
    let mut units = 0u32;
    for (index, c) in line.chars().enumerate() {
        if index >= chars {
            break;
        }
        units += c.len_utf16() as u32;
    }
    units
}

/// 第 `units` 个 UTF-16 code unit 对应的行内字符偏移；落在代理对中间时向后取整，
/// `units` 超过行长度时按行尾算。
pub fn utf16_to_chars(line: &str, units: u32) -> usize {
    let mut seen = 0u32;
    let mut chars = 0usize;
    for c in line.chars() {
        if seen >= units {
            break;
        }
        seen += c.len_utf16() as u32;
        chars += 1;
    }
    chars
}

/// 组装 LSP Position：`line_text` 是文档该行的文本，`char_in_line` 是行内字符偏移。
pub fn position_at(line: u32, line_text: &str, char_in_line: usize) -> Position {
    Position { line, character: chars_to_utf16(line_text, char_in_line) }
}

/// 把 LSP Position 的行内列换回行内字符偏移；`line_text` 是文档该行的文本。
pub fn char_in_line(position: Position, line_text: &str) -> usize {
    utf16_to_chars(line_text, position.character)
}

#[cfg(test)]
mod tests;
