//! 字符在屏幕上占几格。
//!
//! 按单个字符的显示宽度计算（`unicode-width`），不按字素簇合并：在终端里运行的程序按单字符
//! 宽度排光标，按字素簇合并会使光标与字符错位。emoji 的 ZWJ 序列因此占多个字的宽度。

use unicode_width::UnicodeWidthChar;

/// 返回字符 `c` 占的格数：0（组合字符、控制字符）、1 或 2。
pub fn char_width(c: char) -> usize {
    if c.is_ascii() {
        // 可打印 ASCII 占一格；控制字符不会走到这里（解析器已经分出去了），按 0 算。
        return usize::from(c >= ' ' && c != '\x7f');
    }

    c.width().unwrap_or(0).min(2)
}

#[cfg(test)]
mod tests;
