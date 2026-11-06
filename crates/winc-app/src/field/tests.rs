//! `field` 的测试：光标按字素、词、行移动，选区的收起与删除，输入过滤与首行提取，连续键入合并
//! 撤销，输入历史的翻阅与恢复草稿，过时位置落到字符边界，以及绘制几何到字节偏移的映射。

use super::*;

fn field(text: &str) -> TextField {
    let mut field = TextField::default();
    field.insert(text);
    field
}

#[test]
fn caret_moves_by_grapheme_word_and_line() {
    // 「é」是 e 加组合声调，两个字符一个字素；👍🏽 是两个字符一个字素。
    let mut f = field("ae\u{301}👍🏽 foo_bar.baz");
    let len = f.text().len();
    f.move_caret(Unit::Line, false, false);
    assert_eq!(f.caret(), 0);

    f.move_caret(Unit::Char, true, false);
    f.move_caret(Unit::Char, true, false);
    assert_eq!(f.caret(), 4);
    f.move_caret(Unit::Char, true, false);
    assert_eq!(f.caret(), 12);
    f.move_caret(Unit::Char, false, false);
    assert_eq!(f.caret(), 4);

    // emoji 算标点，自成一个词。
    f.move_caret(Unit::Word, true, false);
    assert_eq!(&f.text()[..f.caret()], "ae\u{301}👍🏽");
    f.move_caret(Unit::Word, true, false);
    assert_eq!(&f.text()[f.caret()..], ".baz");

    f.move_caret(Unit::Line, true, false);
    assert_eq!(f.caret(), len);
    f.move_caret(Unit::Word, false, false);
    assert_eq!(&f.text()[f.caret()..], "baz");
    f.move_caret(Unit::Word, false, false);
    assert_eq!(&f.text()[f.caret()..], ".baz");
    f.move_caret(Unit::Word, false, false);
    assert_eq!(&f.text()[f.caret()..], "foo_bar.baz");
}

#[test]
fn selection_and_editing() {
    let mut f = field("hello world");
    f.move_caret(Unit::Word, false, true);
    assert_eq!(f.selected_text(), "world");

    // 有选区时左右箭头先收起选区。
    f.move_caret(Unit::Char, false, false);
    assert_eq!((f.caret(), f.selected_text()), (6, ""));

    f.move_caret(Unit::Line, true, true);
    assert!(f.insert("中文"));
    assert_eq!(f.text(), "hello 中文");

    assert!(f.delete(Unit::Char, false));
    assert!(f.delete(Unit::Word, false));
    assert_eq!(f.text(), "hello ");
    assert!(!f.delete(Unit::Char, true));

    f.select_all();
    assert_eq!(f.selected_text(), "hello ");
    assert!(f.delete(Unit::Char, false));
    assert_eq!(f.text(), "");

    f.insert("a-b c");
    f.select_word_at(1);
    assert_eq!(f.selected_text(), "-");
    f.select_word_at(5);
    assert_eq!(f.selected_text(), "c");
}

#[test]
fn insert_keeps_first_line_and_applies_filter() {
    let mut f = field("ab\ncd");
    assert_eq!(f.text(), "ab");

    let mut goto = TextField::with_filter(|c| c.is_ascii_digit() || c == ':');
    assert!(goto.insert("12:3a"));
    assert!(!goto.insert("abc"));
    assert_eq!(goto.text(), "12:3");

    f.set_text("new");
    assert_eq!(f.selected_text(), "new");

    // 换成相同的文字只全选：不多出撤销步骤，重做记录也还在。
    f.set_text("other");
    assert!(f.undo());
    f.set_text("new");
    assert_eq!(f.selected_text(), "new");
    assert!(f.redo());
    assert_eq!(f.text(), "other");
    assert!(f.undo());
    assert_eq!(f.text(), "new");
}

#[test]
fn typing_merges_into_one_undo_step() {
    let mut f = TextField::default();

    for c in ["f", "o", "o"] {
        f.insert(c);
    }

    f.move_caret(Unit::Char, false, false);
    f.insert("x");
    assert_eq!(f.text(), "foxo");

    assert!(f.undo());
    assert_eq!(f.text(), "foo");
    assert!(f.undo());
    assert_eq!(f.text(), "");
    assert!(!f.undo());

    assert!(f.redo());
    assert!(f.redo());
    assert_eq!((f.text(), f.caret()), ("foxo", 3));
    assert!(!f.redo());
}

#[test]
fn history_recalls_entries_and_restores_the_draft() {
    let mut f = TextField::default();
    f.insert("foo");
    f.remember();
    f.set_text("bar");
    f.remember();
    // 正在输入、还没记下的文字，往回翻时先记下，往后翻还能回来。
    f.set_text("baz");

    assert!(f.recall(true));
    assert_eq!((f.text(), f.caret(), f.selected_text()), ("bar", 3, ""));
    assert!(f.recall(true));
    assert_eq!(f.text(), "foo");
    assert!(!f.recall(true));

    assert!(f.recall(false));
    assert!(f.recall(false));
    assert_eq!(f.text(), "baz");
    // 翻过最新的一条就清空，再往回翻又回到最新的一条。
    assert!(f.recall(false));
    assert_eq!(f.text(), "");
    assert!(!f.recall(false));
    assert!(f.recall(true));
    assert_eq!(f.text(), "baz");

    // 翻历史可以撤销；重复记下的条目挪到最新。
    assert!(f.undo());
    assert_eq!(f.text(), "");
    f.set_text("foo");
    f.remember();
    assert!(f.recall(true));
    assert_eq!(f.text(), "baz");
}

#[test]
fn stale_positions_land_on_a_character_boundary() {
    // 上一帧记录的是「ab」的字素边界，之后文字换成了「中文」：位置 1、2 落在「中」的中间。
    let mut f = field("中文");
    f.set_caret(1, false);
    assert_eq!(f.caret(), 0);
    f.set_caret(5, true);
    assert_eq!((f.caret(), f.selected_text()), (3, "中"));
    f.set_caret(100, false);
    assert_eq!(f.caret(), "中文".len());
    // 在该位置插入文字不会切在字符中间。
    f.set_caret(4, false);
    assert!(f.insert("x"));
    assert_eq!(f.text(), "中x文");
}

#[test]
fn view_maps_x_to_nearest_boundary() {
    let view = FieldView {
        origin: 100.0,
        stops: vec![(0, 0.0), (1, 8.0), (4, 20.0)],
        ..Default::default()
    };
    assert_eq!(view.position_at(90.0), 0);
    assert_eq!(view.position_at(105.0), 1);
    assert_eq!(view.position_at(115.0), 4);
    assert_eq!(view.position_at(500.0), 4);
    assert_eq!(view.x_of(4), 20.0);
}

#[test]
fn multiline_fields_keep_newlines_and_move_by_rows() {
    let mut field = TextField::multiline();
    field.insert("第一行\r\n第二行\rthird");
    assert_eq!(field.text(), "第一行\n第二行\nthird");
    // 单行输入框只取第一行。
    let mut single = TextField::default();
    single.insert("a\nb");
    assert_eq!(single.text(), "a");
    // 开头是换行（例如从终端复制时带上了上一行的行尾）时取第一个非空行。
    let mut single = TextField::default();
    single.insert("\r\nfoo\nbar");
    assert_eq!(single.text(), "foo");
    // 单独键入的空格是内容；只有空白的多行内容仍然不插入。
    let mut single = TextField::default();
    single.insert("Maple");
    assert!(single.insert(" "));
    single.insert("Mono");
    assert_eq!(single.text(), "Maple Mono");
    assert!(!single.insert(" \n "));

    // Home、End 到这一行（按换行符分）的两端。
    let second = "第一行\n".len();
    field.set_caret(second + 3, false);
    field.move_caret(Unit::Line, false, false);
    assert_eq!(field.caret(), second);
    field.move_caret(Unit::Line, true, false);
    assert_eq!(field.caret(), second + "第二行".len());

    // 上下移动按绘制时记下的行：三行，每个字素宽 10。
    let mut stops = Vec::new();
    let mut rows = Vec::new();
    let (mut x, mut row) = (0.0, 0);
    stops.push((0, 0.0));
    rows.push(0);
    for (start, g) in field.text().grapheme_indices(true) {
        if g == "\n" {
            row += 1;
            x = 0.0;
        } else {
            x += 10.0;
        }
        stops.push((start + g.len(), x));
        rows.push(row);
    }
    field.view = FieldView {
        stops,
        rows,
        line_height: 20.0,
        ..FieldView::default()
    };
    field.set_caret("第一".len(), false);
    assert!(field.move_vertical(true, false));
    assert_eq!(field.caret(), second + "第二".len());
    assert!(field.move_vertical(true, false));
    assert_eq!(&field.text()[field.caret() - 2..field.caret()], "th");
    assert!(!field.move_vertical(true, false));
    // 点在第二行中间。
    assert_eq!(
        field.view.position_at_point(11.0, 25.0),
        second + "第".len()
    );
}

#[test]
fn editing_a_recalled_entry_stops_browsing_history() {
    let mut field = TextField::multiline();
    field.insert("first");
    field.remember();
    field.set_text("");
    assert!(field.recall(true));
    assert_eq!(field.text(), "first");
    assert!(field.recalling());
    field.insert(" edited");
    assert!(!field.recalling());
}

/// 回归测试：撤销栈每一步都存整段文字，只按步数限制时，粘贴一大段文字后每改一次都会在栈里
/// 多存一段同样长的文字。
#[test]
fn undo_keeps_a_bounded_amount_of_text() {
    let mut f = field(&"x".repeat(200 << 10));
    for _ in 0..50 {
        f.insert("ab");
        f.insert(" ");
    }
    let bytes: usize = f.undo.iter().map(|s| s.text.len()).sum();
    assert!(bytes <= UNDO_BYTES + (200 << 10) + 200, "{bytes}");
    assert!(f.undo.len() < 50);
    // 最近的几步仍然能撤销。
    let before = f.text().to_owned();
    assert!(f.undo());
    assert_ne!(f.text(), before);
    // 一步就超过上限时也保留这一步。
    let mut f = field("a");
    f.select_all();
    f.insert(&"y".repeat(UNDO_BYTES * 2));
    f.select_all();
    f.insert("b");
    assert!(f.undo());
    assert_eq!(f.text().len(), UNDO_BYTES * 2);
}
