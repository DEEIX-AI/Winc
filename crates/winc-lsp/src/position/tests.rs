//! 行内 char ↔ UTF-16 code unit 换算的测试：ASCII、中文、emoji（代理对）、组合字符、
//! 超长行与越界钳制。

use super::*;

#[test]
fn ascii_is_one_unit_per_char() {
    assert_eq!(chars_to_utf16("fmt.Println", 4), 4);
    assert_eq!(utf16_to_chars("fmt.Println", 4), 4);
    // 与 LSP Position 的组装/拆开往返。
    let line = "\tfmt.Pri";
    let position = position_at(5, line, 8);
    assert_eq!(position, Position { line: 5, character: 8 });
    assert_eq!(char_in_line(position, line), 8);
}

#[test]
fn chinese_is_one_unit_per_char() {
    // 中文字符都在 BMP 内：一个字符一个 code unit。
    let line = "你好世界";
    assert_eq!(chars_to_utf16(line, 2), 2);
    assert_eq!(utf16_to_chars(line, 2), 2);
    let position = position_at(0, line, 3);
    assert_eq!(char_in_line(position, line), 3);
}

#[test]
fn emoji_is_surrogate_pair() {
    // 😀（U+1F600）在 BMP 外：一个字符两个 code unit。
    let line = "a😀b";
    assert_eq!(chars_to_utf16(line, 2), 3, "a + emoji = 1 + 2");
    assert_eq!(utf16_to_chars(line, 3), 2);
    // 代理对中间（1 个 unit 在 emoji 内部）：向后取整到 emoji 之后。
    assert_eq!(utf16_to_chars(line, 2), 2);
    assert_eq!(chars_to_utf16(line, 3), 4, "整行");
}

#[test]
fn combining_characters_count_separately() {
    // `é` 写成 `e` + 组合重音（U+0301）：两个字符、两个 code unit（都在 BMP）。
    let line = "e\u{301}x";
    assert_eq!(line.chars().count(), 3);
    assert_eq!(chars_to_utf16(line, 2), 2);
    assert_eq!(utf16_to_chars(line, 2), 2);
    assert_eq!(position_at(0, line, 2).character, 2);
}

#[test]
fn mixed_line_round_trip() {
    // 中文、emoji、ASCII 混排：每个字符偏移往返一致。
    let line = "函数 f(😀) 调用";
    let count = line.chars().count();
    for chars in 0..=count {
        let units = chars_to_utf16(line, chars);
        assert_eq!(utf16_to_chars(line, units), chars, "往返偏移 {chars}");
    }
}

#[test]
fn out_of_range_is_clamped_to_line_end() {
    let line = "abc";
    assert_eq!(chars_to_utf16(line, 100), 3);
    assert_eq!(utf16_to_chars(line, 100), 3);
    assert_eq!(chars_to_utf16("", 5), 0);
    assert_eq!(utf16_to_chars("", 5), 0);
    // units = 0 是行首。
    assert_eq!(utf16_to_chars(line, 0), 0);
}

#[test]
fn long_line() {
    // 超长行：结尾位置的换算正确（大文件常见：压缩过的 JSON 一行）。
    let line = "x".repeat(10_000);
    assert_eq!(chars_to_utf16(&line, 10_000), 10_000);
    assert_eq!(utf16_to_chars(&line, 10_000), 10_000);

    let mixed = format!("{}😀{}", "a".repeat(5_000), "b".repeat(5_000));
    assert_eq!(mixed.chars().count(), 10_001);
    // emoji 之后 5001 个字符 = 5002 个 code unit（5000 + 代理对 2 个）。
    assert_eq!(chars_to_utf16(&mixed, 5_001), 5_002);
    assert_eq!(utf16_to_chars(&mixed, 5_002), 5_001);
}
