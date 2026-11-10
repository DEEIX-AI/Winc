//! LSP 类型转换：Rope 位置 ↔ LSP Position。

use crate::{Error, OffsetEncoding, Result};
use ropey::Rope;

/// 将 Rope 中的字符偏移转换为 LSP Position。
pub fn pos_to_lsp_pos(rope: &Rope, pos: usize, encoding: OffsetEncoding) -> Result<lsp_types::Position> {
    if pos > rope.len_chars() {
        return Err(Error::PositionConversion(format!(
            "Position {} out of bounds (len: {})",
            pos,
            rope.len_chars()
        )));
    }

    let line = rope.char_to_line(pos);
    let line_start_char = rope.line_to_char(line);

    let character = match encoding {
        OffsetEncoding::Utf8 => {
            let line_start_byte = rope.char_to_byte(line_start_char);
            let pos_byte = rope.char_to_byte(pos);
            (pos_byte - line_start_byte) as u32
        }
        OffsetEncoding::Utf16 => {
            let line_start_utf16 = rope.char_to_utf16_cu(line_start_char);
            let pos_utf16 = rope.char_to_utf16_cu(pos);
            (pos_utf16 - line_start_utf16) as u32
        }
        OffsetEncoding::Utf32 => (pos - line_start_char) as u32,
    };

    Ok(lsp_types::Position::new(line as u32, character))
}

/// 将 LSP Position 转换为 Rope 中的字符偏移。
pub fn lsp_pos_to_pos(rope: &Rope, pos: lsp_types::Position, encoding: OffsetEncoding) -> Result<usize> {
    let line = pos.line as usize;

    // 检查行号是否有效
    if line >= rope.len_lines() {
        // LSP 协议允许 line 等于总行数（表示文件末尾）
        if line == rope.len_lines() {
            return Ok(rope.len_chars());
        }
        return Err(Error::PositionConversion(format!(
            "Line {} out of bounds (len: {})",
            line,
            rope.len_lines()
        )));
    }

    let line_start_char = rope.line_to_char(line);
    let line_end_char = if line + 1 < rope.len_lines() {
        rope.line_to_char(line + 1)
    } else {
        rope.len_chars()
    };

    // 根据编码方式计算字符偏移
    let offset = match encoding {
        OffsetEncoding::Utf8 => {
            let line_start_byte = rope.char_to_byte(line_start_char);
            let line_end_byte = rope.char_to_byte(line_end_char);
            let target_byte = line_start_byte + pos.character as usize;

            // 检查字节偏移是否超出行尾
            if target_byte > line_end_byte {
                return Ok(line_end_char);
            }

            rope.try_byte_to_char(target_byte)
                .map_err(|_| Error::PositionConversion("Invalid UTF-8 byte offset".to_string()))?
        }
        OffsetEncoding::Utf16 => {
            let line_start_utf16 = rope.char_to_utf16_cu(line_start_char);
            let line_end_utf16 = rope.char_to_utf16_cu(line_end_char);
            let target_utf16 = line_start_utf16 + pos.character as usize;

            // 检查 UTF-16 偏移是否超出行尾
            if target_utf16 > line_end_utf16 {
                return Ok(line_end_char);
            }

            rope.try_utf16_cu_to_char(target_utf16)
                .map_err(|_| Error::PositionConversion("Invalid UTF-16 offset".to_string()))?
        }
        OffsetEncoding::Utf32 => {
            let target_char = line_start_char + pos.character as usize;

            // 检查字符偏移是否超出行尾
            if target_char > line_end_char {
                return Ok(line_end_char);
            }

            target_char
        }
    };

    Ok(offset)
}

/// 将 Rope 中的字符范围转换为 LSP Range。
pub fn range_to_lsp_range(
    rope: &Rope,
    start: usize,
    end: usize,
    encoding: OffsetEncoding,
) -> Result<lsp_types::Range> {
    let lsp_start = pos_to_lsp_pos(rope, start, encoding)?;
    let lsp_end = pos_to_lsp_pos(rope, end, encoding)?;
    Ok(lsp_types::Range::new(lsp_start, lsp_end))
}

/// 将 LSP Range 转换为 Rope 中的字符范围。
pub fn lsp_range_to_range(
    rope: &Rope,
    range: lsp_types::Range,
    encoding: OffsetEncoding,
) -> Result<(usize, usize)> {
    let start = lsp_pos_to_pos(rope, range.start, encoding)?;
    let end = lsp_pos_to_pos(rope, range.end, encoding)?;

    // 确保 start <= end
    if start > end {
        return Err(Error::PositionConversion(format!(
            "Invalid range: start {} > end {}",
            start, end
        )));
    }

    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pos_conversion_basic() {
        let rope = Rope::from_str("Hello\nWorld");
        let encoding = OffsetEncoding::Utf16;

        // 行首
        let pos = pos_to_lsp_pos(&rope, 0, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(0, 0));

        // 行尾
        let pos = pos_to_lsp_pos(&rope, 5, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(0, 5));

        // 第二行开头
        let pos = pos_to_lsp_pos(&rope, 6, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(1, 0));

        // 文件末尾
        let pos = pos_to_lsp_pos(&rope, 11, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(1, 5));
    }

    #[test]
    fn test_pos_roundtrip() {
        let rope = Rope::from_str("Hello\nWorld\n");
        let encoding = OffsetEncoding::Utf16;

        for i in 0..=rope.len_chars() {
            let lsp_pos = pos_to_lsp_pos(&rope, i, encoding).unwrap();
            let back = lsp_pos_to_pos(&rope, lsp_pos, encoding).unwrap();
            assert_eq!(i, back, "Roundtrip failed for position {}", i);
        }
    }

    #[test]
    fn test_range_conversion() {
        let rope = Rope::from_str("Hello\nWorld");
        let encoding = OffsetEncoding::Utf16;

        let range = range_to_lsp_range(&rope, 1, 4, encoding).unwrap();
        assert_eq!(range.start, lsp_types::Position::new(0, 1));
        assert_eq!(range.end, lsp_types::Position::new(0, 4));

        let (start, end) = lsp_range_to_range(&rope, range, encoding).unwrap();
        assert_eq!((start, end), (1, 4));
    }

    #[test]
    fn test_utf8_encoding() {
        let rope = Rope::from_str("你好世界");
        let encoding = OffsetEncoding::Utf8;

        // "你" 占 3 字节
        let pos = pos_to_lsp_pos(&rope, 1, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(0, 3));

        // "你好" 占 6 字节
        let pos = pos_to_lsp_pos(&rope, 2, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(0, 6));
    }

    #[test]
    fn test_utf16_encoding() {
        let rope = Rope::from_str("你好世界");
        let encoding = OffsetEncoding::Utf16;

        // 每个中文字符占 1 个 UTF-16 code unit
        let pos = pos_to_lsp_pos(&rope, 1, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(0, 1));

        let pos = pos_to_lsp_pos(&rope, 2, encoding).unwrap();
        assert_eq!(pos, lsp_types::Position::new(0, 2));
    }
}
