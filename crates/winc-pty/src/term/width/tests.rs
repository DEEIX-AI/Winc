//! `width` 的测试：各类字符占用的格数。

use super::*;

#[test]
fn widths_follow_wcwidth() {
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('中'), 2);
    assert_eq!(char_width('😀'), 2);
    assert_eq!(char_width('\u{301}'), 0); // 组合重音，U+0301
    assert_eq!(char_width('\u{200d}'), 0); // ZWJ
    assert_eq!(char_width('\x07'), 0);
}
