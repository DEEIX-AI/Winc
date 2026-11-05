//! `minimap` 的测试：一个显示行如何切成小块。

use super::*;

#[test]
fn blanks_split_strokes_and_wide_text_counts_double() {
    let chars: Vec<char> = "\tab  中".chars().collect();
    let mut cells = Vec::new();
    let whole = 0..chars.len();
    strokes((&chars, &[]), (whole, 0), 4, 80, |range, _| {
        cells.push(range)
    });
    assert_eq!(cells, [4..6, 8..10]);

    // 折行后的一段从自己的缩进开始，只取这一段的字符。
    let mut cells = Vec::new();
    strokes((&chars, &[]), (2..3, 6), 4, 80, |range, _| {
        cells.push(range)
    });
    assert_eq!(cells, vec![6..7]);

    // 超出缩略图宽度的部分不画。
    let mut cells = Vec::new();
    strokes((&['x'; 100], &[]), (0..100, 0), 4, 10, |range, _| {
        cells.push(range)
    });
    assert_eq!(cells, vec![0..10]);
}
