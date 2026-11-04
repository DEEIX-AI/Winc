//! `wrap` 模块的测试：空白与标点处的断行、中日韩文字的断行禁则，以及行内位置与显示列的
//! 换算。

use super::*;

fn split(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let b = breaks(&chars, width, 4);
    (0..b.rows())
        .map(|r| chars[b.range(r, chars.len())].iter().collect())
        .collect()
}

#[test]
fn breaks_after_spaces_and_keeps_the_indent() {
    assert_eq!(split("short", 10), ["short"]);
    assert_eq!(split("hello world again", 11), ["hello world ", "again"]);
    assert_eq!(split("hello world again", 9), ["hello ", "world ", "again"]);
    // 后续各段缩进到与第一段缩进相同的显示列。
    let chars: Vec<char> = "    let value = compute(alpha, beta);".chars().collect();
    let b = breaks(&chars, 20, 4);
    assert_eq!(b.indent, 4);
    assert!(b.rows() >= 2);
    for r in 0..b.rows() {
        let seg = b.range(r, chars.len());
        let cells = b.indent_of(r) + seg.len();
        assert!(cells <= 20, "第 {r} 段 {cells} 格");
    }
    // 单词比行宽还长时在行宽处硬断。
    assert_eq!(split("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    // 缩进之后是超长单词：第一段不能只剩缩进，单词从第一段起就硬断。
    assert_eq!(
        split("    aaaaaaaaaaaaaaaaaaaa", 10),
        ["    aaaaaa", "aaaaaa", "aaaaaa", "aa"]
    );
}

#[test]
fn chinese_breaks_anywhere_but_not_before_closing_punctuation() {
    // 每个汉字占 2 个显示列，行宽 6 放 3 个字；「，」不能在行首，与前一个字一起移到下一段。
    assert_eq!(split("一二三四五六", 6), ["一二三", "四五六"]);
    assert_eq!(split("一二三，四五", 6), ["一二", "三，四", "五"]);
    // 全角左括号不能在行末。
    assert_eq!(split("一二（三四", 6), ["一二", "（三四"]);
}

#[test]
fn code_breaks_after_punctuation_and_positions_map_back() {
    let parts = split("call(alpha,beta,gamma)", 12);
    assert_eq!(parts.concat(), "call(alpha,beta,gamma)");
    assert!(parts.iter().all(|p| p.chars().count() <= 12));
    assert_eq!(parts[0], "call(alpha,");

    let text = "  aaaa bbbb cccc";
    let chars: Vec<char> = text.chars().collect();
    let b = breaks(&chars, 8, 4);
    assert_eq!(b.starts, vec![7, 12]);
    assert_eq!((b.row_of(6), b.row_of(7), b.row_of(15)), (0, 1, 2));
    let slice = RopeSlice::from(text);
    // 第二段从缩进处（2 个显示列）开始。
    assert_eq!(cell_in_row(slice, &b, 7, 4), 2);
    assert_eq!(cell_in_row(slice, &b, 9, 4), 4);
    // 缩进超过行宽的一半时不缩进；行宽为 0 时按 1 处理，字符逐个硬断。
    assert_eq!(
        breaks(&"        abcdefgh".chars().collect::<Vec<_>>(), 10, 4).indent,
        0
    );
    assert_eq!(breaks(&['a', 'b'], 0, 4).rows(), 2);
}
