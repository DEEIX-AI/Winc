//! 行号栏改动标记的测试：按行范围取原文时保留各种换行符。

use super::*;

#[test]
fn lines_keep_their_endings() {
    let text = "a\nb\r\nc\rd";
    assert_eq!(lines_of(text, 0..1), "a\n");
    assert_eq!(lines_of(text, 1..3), "b\r\nc\r");
    assert_eq!(lines_of(text, 3..4), "d");
    assert_eq!(lines_of(text, 2..2), "");
    assert_eq!(lines_of(text, 0..9), text);
    assert_eq!(lines_of("a\n\nb", 1..3), "\nb");
    assert_eq!(lines_of("a\r\r\nb\n", 1..2), "\r\n");
    assert_eq!(lines_of("a\n", 1..2), "");
}
