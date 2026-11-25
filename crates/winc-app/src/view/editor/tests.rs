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
fn occurrences_of_the_word_under_the_caret() {    let mut doc = Document::from_text(DocumentId(1), "let id = width + id_x + id;\n");
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

/// 锚定诊断文字的气泡：默认在诊断文字所在行上方；上方放不下翻到下方；横向不越出编辑区；
/// 多行卡片按实际高度参与摆放。
#[test]
fn the_diagnostic_bubble_sits_above_the_error() {
    let editor = Rect::new(100.0, 40.0, 600.0, 400.0);
    let line_h = 20.0;
    // 空间充足：气泡在错误行上方，底边与锚点留出间隙。
    let (x, y, above) = hint::place_above_name(300.0, 200.0, line_h, editor, 200.0, hint::HEIGHT);
    assert!(above, "空间充足时在错误行上方");
    assert!(y + hint::HEIGHT < 200.0, "气泡底边在锚点之上");
    assert_eq!(x, 280.0, "卡片左缘相对锚点略偏左");
    assert!(x >= editor.x + 8.0 && x + 200.0 <= editor.right());

    // 靠近编辑区顶部：翻到错误行下方。
    let (_, y, above) = hint::place_above_name(300.0, 60.0, line_h, editor, 200.0, hint::HEIGHT);
    assert!(!above, "上方放不下时翻到下方");
    assert!(y > 60.0, "气泡上沿在锚点之下");

    // 多行消息的卡片更高：同样的位置上方放不下，改到下方。
    let tall = 120.0;
    let (_, y, above) = hint::place_above_name(300.0, 160.0, line_h, editor, 200.0, tall);
    assert!(!above, "高卡片在上方放不下");
    assert!(y >= 160.0);

    // 靠近右边缘：卡片夹在编辑区内，不越出右界。
    let (x, _, _) = hint::place_above_name(690.0, 200.0, line_h, editor, 200.0, hint::HEIGHT);
    assert!(x + 200.0 <= editor.right(), "{x}");

    // 靠近左边缘：不越出左界。
    let (x, _, _) = hint::place_above_name(90.0, 200.0, line_h, editor, 200.0, hint::HEIGHT);
    assert_eq!(x, editor.x + 8.0);
}

/// 诊断气泡的折行：按宽度切行、保留原文的空格断点，超出行数时截断并在末行加省略号。
#[test]
fn diagnostic_messages_wrap_into_bounded_lines() {
    // 每 10 逻辑像素算 1 个字符宽：40 宽 → 4 字符一行。
    let measure = |s: &str| s.chars().count() as f32 * 10.0;
    let lines = hint::wrap_message("aaaa bbbb cccc", measure, 40.0, 4);
    assert_eq!(lines, vec!["aaaa", "bbbb", "cccc"]);

    // 单词比行宽还长：硬切。
    let lines = hint::wrap_message("aaaaaaaaaaaa", measure, 40.0, 4);
    assert_eq!(lines, vec!["aaaa", "aaaa", "aaaa"]);

    // 超出行数：末行加省略号。
    let lines = hint::wrap_message("aaaa bbbb cccc dddd eeee", measure, 40.0, 2);
    assert_eq!(lines.len(), 2);
    assert!(lines[1].ends_with('…'), "{lines:?}");

    // 原有的换行（多条诊断以 \n 连接）各自成段。
    let lines = hint::wrap_message("aaaa\nbbbb", measure, 40.0, 4);
    assert_eq!(lines, vec!["aaaa", "bbbb"]);

    // 空消息给出一行空串，不留空卡片。
    assert_eq!(hint::wrap_message("", measure, 40.0, 4), vec![""]);
}

/// 气泡宽度：不窄于最小可用宽度，不长于编辑区 60%；短消息取两者中能容纳内容的较窄者。
#[test]
fn the_bubble_width_is_bounded() {
    let measure = |s: &str| s.chars().count() as f32 * 10.0;
    let wide = Rect::new(0.0, 0.0, 1000.0, 800.0);
    let room = hint::PAD * 2.0 + hint::ICON_ROOM;
    // 短消息：至少 180 宽（含留白与图标列）。
    let w = hint::bubble_width(measure("错误"), room, wide);
    assert_eq!(w, 180.0);
    // 长消息：不超过编辑区的 60%。
    let w = hint::bubble_width(measure(&"x".repeat(500)), room, wide);
    assert_eq!(w, 600.0);
    // 编辑区很窄：不窄于最小可用宽度，也不超过编辑区宽度。
    let narrow = Rect::new(0.0, 0.0, 250.0, 800.0);
    let w = hint::bubble_width(measure(&"x".repeat(500)), room, narrow);
    assert!(w <= 250.0, "{w}");
    assert!(w >= 180.0, "{w}");
}
