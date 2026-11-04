//! 编辑区绘制的测试：折行后选区跨段的绘制范围，以及光标下的词与选区的全部出现位置。

use winc_core::Selection;
use winc_editor::{Document, DocumentId};

use super::*;

fn found(doc: &Document, line: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    Occurrences::of(doc)
        .map(|o| o.find(&chars).collect())
        .unwrap_or_default()
}

/// 返回断成多段的一行中，列 `from..to` 落在哪几段的哪几格，每项为（段号，起点格，终点格）。
fn spans_of(line: &str, width: usize, (from, to): (usize, usize)) -> Vec<(usize, usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let breaks = winc_core::wrap::breaks(&chars, width, 4);
    let mut out = Vec::new();
    wrapped_spans(&chars, &breaks, (from, to), 4, |part, a, b| {
        out.push((part, a, b))
    });
    out
}

#[test]
fn wrapped_spans_end_where_each_row_ends() {
    // 分为「hello world 」和「again」两段：跨过分界的那一块绘制到第一段的末尾，而不是回到
    // 下一段的开头。
    let line = "hello world again";
    assert_eq!(spans_of(line, 11, (6, 14)), [(0, 6, 12), (1, 0, 2)]);
    // 正好在分界处结束：只有第一段的那一块，第二段开头不会多出一块。
    assert_eq!(spans_of(line, 11, (6, 12)), [(0, 6, 12)]);
    // 零宽的范围位于分界处：绘制在下一段的开头，与光标的位置一致。
    assert_eq!(spans_of(line, 11, (12, 12)), [(1, 0, 0)]);
    // 终点超过行尾时，绘制到行尾为止。
    assert_eq!(spans_of(line, 11, (13, 100)), [(1, 1, 5)]);

    // 中日韩文字每个字符占两格；后续各段与第一段缩进相同。
    assert_eq!(
        spans_of("中文中文中文", 5, (1, 5)),
        [(0, 2, 4), (1, 0, 4), (2, 0, 2)]
    );
    let line = "    let value = compute(alpha, beta);";
    let chars: Vec<char> = line.chars().collect();
    let breaks = winc_core::wrap::breaks(&chars, 20, 4);
    assert!(breaks.rows() >= 2 && breaks.indent == 4);
    let whole = spans_of(line, 20, (0, chars.len()));
    assert_eq!(whole.len(), breaks.rows());
    for (part, a, b) in whole {
        let range = breaks.range(part, chars.len());
        assert_eq!(a, breaks.indent_of(part));
        assert_eq!(b, a + range.len(), "第 {part} 段");
    }
}

#[test]
fn occurrences_of_the_word_under_the_caret() {
    let mut doc = Document::from_text(DocumentId(1), "let id = width + id_x + id;\n");
    // 光标在 `id` 的末尾（右边是空格）时也算在词上。
    doc.selection = Selection::point(6);
    let line = "let id = width + id_x + id;";
    assert_eq!(found(&doc, line), [(4, 6), (24, 26)]);
    let first = 0..1;
    let count = Occurrences::of(&doc)
        .unwrap()
        .count(doc.text(), std::slice::from_ref(&first));
    assert_eq!(count, 2);

    // 光标在空白处或数字上：不标注。
    doc.selection = Selection::point(8);
    assert!(Occurrences::of(&doc).is_none());
    let mut digits = Document::from_text(DocumentId(2), "x = 10 + 10");
    digits.selection = Selection::point(5);
    assert!(Occurrences::of(&digits).is_none());
}

#[test]
fn occurrences_of_a_selection_match_any_text_but_skip_itself() {
    let mut doc = Document::from_text(DocumentId(1), "abcab ab\nab");
    doc.selection = Selection::new(0, 2);
    let o = Occurrences::of(&doc).unwrap();
    assert_eq!(o.skip, Some((0, 2)));
    assert_eq!(found(&doc, "abcab ab"), [(0, 2), (3, 5), (6, 8)]);

    // 跨行、单个字符、全是空白的选区：不标注。
    doc.selection = Selection::new(6, 10);
    assert!(Occurrences::of(&doc).is_none());
    doc.selection = Selection::new(0, 1);
    assert!(Occurrences::of(&doc).is_none());
}

/// 回归：光标所在的词不限长度，停在一长串没有分隔的字符里时，每个可见行都拿它逐处比较。
#[test]
fn long_words_under_the_caret_are_not_marked() {
    let long = "x".repeat(10_000);
    let mut doc = Document::from_text(DocumentId(1), &format!("{long} {long}\n"));
    doc.selection = Selection::point(5_000);
    assert!(Occurrences::of(&doc).is_none());
    // 上限以内的词仍然标注。
    let word = "y".repeat(200);
    let mut doc = Document::from_text(DocumentId(2), &format!("{word} {word}\n"));
    doc.selection = Selection::point(3);
    assert!(Occurrences::of(&doc).is_some());
}

/// 回归：每帧把全部其余选区逐个与可见范围比较（光标可能有上万个）。
///
/// 二分找出的一段与逐个比较的结果相同。
#[test]
fn visible_selections_match_a_linear_filter() {
    let mut seed = 0x5eed_u64;
    let mut next = |n: u64| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((seed >> 33) % n) as usize
    };
    for _ in 0..2_000 {
        // 按位置排好、互不重叠的选区（可以是光标）。
        let mut all = Vec::new();
        let mut at = next(3);
        for _ in 0..next(12) {
            let len = next(4);
            all.push(Selection::new(at, at + len));
            at += len + 1 + next(3);
        }
        let start = next(at as u64 + 2);
        let shown = start..=start + next(10);
        let expected: Vec<Selection> = all
            .iter()
            .filter(|s| s.to() >= *shown.start() && s.from() <= *shown.end())
            .copied()
            .collect();
        assert_eq!(
            area::visible_selections(&all, &shown),
            expected,
            "{all:?} {shown:?}"
        );
    }
}
