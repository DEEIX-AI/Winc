//! 终端历史行与选区的测试：行号映射、按词与按行复制、折叠后的显示行。

use super::*;
use winc_pty::protocol::Block;

fn tab_with(screen: &[&str], history: &[&str]) -> TermTab {
    let size = Size {
        cols: 20,
        rows: screen.len() as u16,
        width: 0,
        height: 0,
    };
    let mut tab = TermTab::new(1, size);
    let line = |s: &&str| Line {
        text: s.to_string(),
        ..Line::default()
    };
    tab.rows = screen.iter().map(line).collect();
    tab.history = (100, 100 + history.len() as u64);
    tab.store_history(100, history.iter().map(line).collect());
    tab
}

#[test]
fn lines_are_numbered_across_history_and_screen() {
    let tab = tab_with(&["$ ls", "a b"], &["old 1", "old 2"]);
    assert_eq!(tab.top(), 102);
    assert_eq!(tab.line(101).unwrap().text, "old 2");
    assert_eq!(tab.line(103).unwrap().text, "a b");
    assert!(tab.line(104).is_none());
    assert_eq!(tab.max_scroll(), 2);
}

#[test]
fn new_output_keeps_a_scrolled_view_still() {
    let mut tab = tab_with(&["a", "b"], &["h1", "h2", "h3"]);
    tab.scroll = 2;
    let top = tab.top();
    tab.apply(Frame {
        cols: 20,
        rows: 2,
        scroll: 1,
        lines: vec![(1, Line::default())],
        history: (100, 104),
        ..Frame::default()
    });
    assert_eq!(tab.scroll, 3);
    assert_eq!(tab.top(), top);
}

#[test]
fn selections_expand_by_word_and_line_and_copy_across_rows() {
    let mut tab = tab_with(&["see src/main.rs:12 ok", "next line"], &[]);
    let p = |line, col| Point { line, col };
    assert_eq!(tab.word_at(p(100, 6)), Some((p(100, 4), p(100, 18))));
    assert!(tab.word_at(p(100, 3)).is_none());

    tab.selection = Some(Selection {
        anchor: p(100, 6),
        head: p(100, 6),
        unit: Unit::Word,
    });
    assert_eq!(tab.selected_text().unwrap().unwrap(), "src/main.rs:12");

    tab.selection = Some(Selection {
        anchor: p(101, 2),
        head: p(100, 19),
        unit: Unit::Char,
    });
    assert_eq!(tab.selected_text().unwrap().unwrap(), "ok\nne");

    tab.selection = Some(Selection {
        anchor: p(100, 3),
        head: p(101, 0),
        unit: Unit::Line,
    });
    assert_eq!(
        tab.selected_text().unwrap().unwrap(),
        "see src/main.rs:12 ok\nnext line"
    );
    assert_eq!(tab.selected_cols(101), Some((0, usize::MAX)));
}

#[test]
fn soft_wrapped_rows_copy_as_one_line() {
    let mut tab = tab_with(&["abcde", "fg"], &[]);
    tab.rows[0].wrapped = true;
    tab.select_all();
    assert_eq!(tab.selected_text().unwrap().unwrap(), "abcdefg");
}

#[test]
fn a_wide_character_wrapped_to_the_next_row_copies_without_the_filler() {
    use winc_pty::protocol::{Run, Style};

    // 「界」在上一行放不下而换到下一行：上一行末尾是带 `FILLER` 的占位空格。
    let mut tab = tab_with(&["abc ", "界d"], &[]);
    let filler = Style {
        attrs: Attrs(Attrs::FILLER),
        ..Style::default()
    };
    tab.rows[0].wrapped = true;
    tab.rows[0].runs = vec![
        Run {
            start: 0,
            style: Style::default(),
        },
        Run {
            start: 3,
            style: filler,
        },
    ];
    tab.select_all();
    assert_eq!(tab.selected_text().unwrap().unwrap(), "abc界d");
}

#[test]
fn lines_gone_from_the_scrollback_are_not_asked_for_again() {
    let mut tab = tab_with(&["x"], &["a", "b"]);
    // 选区从已经移除的第 90 行开始：只算还在的行，不去要第 90 行。
    tab.selection = Some(Selection {
        anchor: Point { line: 90, col: 0 },
        head: Point {
            line: 102,
            col: usize::MAX,
        },
        unit: Unit::Char,
    });
    assert_eq!(tab.selected_text().unwrap().as_deref(), Some("a\nb\nx"));
    // 缺的行取过一次还没有：跳过它们复制，不再要。
    tab.cache.remove(&101);
    assert_eq!(tab.selected_text(), Err((101, 102)));
    assert_eq!(tab.selected_text_lossy().as_deref(), Some("a\nx"));
}

#[test]
fn copying_uncached_history_asks_for_it_first() {
    let mut tab = tab_with(&["x"], &["a", "b"]);
    tab.cache.clear();
    tab.select_all();
    assert_eq!(tab.selected_text(), Err((100, 102)));
}

#[test]
fn display_rows_and_lines_map_back_and_forth_with_folds() {
    let screen: Vec<String> = (0..6).map(|i| format!("s{i}")).collect();
    let history: Vec<String> = (0..30).map(|i| format!("h{i}")).collect();
    let screen: Vec<&str> = screen.iter().map(String::as_str).collect();
    let history: Vec<&str> = history.iter().map(String::as_str).collect();
    let mut tab = tab_with(&screen, &history);
    // 行号 100..136：三个块，第一个从回滚缓冲开头之前就开始了，第三个折到屏幕上。
    let block = |id, prompt, output, end| Block {
        id,
        prompt,
        command: prompt,
        output: Some(output),
        end: Some(end),
        exit: Some(0),
        ..Block::default()
    };
    tab.blocks = vec![
        block(1, 90, 95, 110),
        block(2, 110, 111, 120),
        block(3, 120, 121, 134),
    ];
    tab.folded = vec![1, 2, 3];
    let len = tab.display_len();
    // 136 行，藏起 (110-100-1) + (120-111-1) + (134-121-1) = 9 + 8 + 12 行。
    assert_eq!(len, 36 - 29);
    for d in 0..len {
        let row = tab.row_at(d).unwrap();
        assert_eq!(tab.display_of(row.line(&tab)), d, "{d} {row:?}");
    }
    assert_eq!(tab.row_at(len), None);
    // 折起的那一段里的任何一行都落在那一段的显示行上。
    assert_eq!(tab.display_of(125), tab.display_of(121));
    // 看最新输出时网格从后往前排，最后一行是屏幕最后一行。
    assert_eq!(tab.view().last(), Some(&ViewRow::Line(135)));
    // 翻到最上面再展开：第一行不动。
    tab.scroll = tab.max_scroll();
    let top = tab.top();
    tab.set_folded(2, false);
    assert_eq!(tab.top(), top);
}

#[test]
fn find_reports_cell_ranges() {
    let line = Line {
        text: "中文 Error error".into(),
        ..Line::default()
    };
    assert_eq!(
        find_in_line(&line.text, "error", false),
        [(5, 10), (11, 16)]
    );
    assert_eq!(find_in_line(&line.text, "Error", true), [(5, 10)]);
    assert_eq!(cells_text(&line, 0, 4), "中文");
    // 从「中」的后半格选起：「中」也复制上。
    assert_eq!(cells_text(&line, 1, 3), "中文");
}

#[test]
fn the_alternate_screen_does_not_scroll_into_the_shell_history() {
    let mut tab = tab_with(&["vim", "~"], &["old 1", "old 2"]);
    tab.modes = Modes(Modes::ALT_SCREEN);
    assert_eq!(tab.max_scroll(), 0);
    tab.scroll_to(100, 0);
    assert_eq!(tab.scroll, 0);
}

#[test]
fn folding_the_output_at_the_top_brings_its_command_into_view() {
    let screen: Vec<String> = (0..6).map(|i| format!("s{i}")).collect();
    let history: Vec<String> = (0..30).map(|i| format!("h{i}")).collect();
    let screen: Vec<&str> = screen.iter().map(String::as_str).collect();
    let history: Vec<&str> = history.iter().map(String::as_str).collect();
    let mut tab = tab_with(&screen, &history);
    tab.blocks = vec![Block {
        id: 1,
        prompt: 110,
        command: 110,
        output: Some(111),
        end: Some(125),
        exit: Some(0),
        ..Block::default()
    }];
    // 网格第一行在输出中间，命令行吸顶。
    tab.scroll_to(115, 0);
    assert_eq!(tab.sticky(), Some(0));

    // 折起：命令行和「⋯ 13 行」都看得见，不被吸顶的命令行盖住。
    tab.set_folded(1, true);
    assert_eq!(tab.top(), 110);
    assert_eq!(tab.sticky(), None);
    assert!(matches!(tab.view()[1], ViewRow::Fold(0, _)));
}

#[test]
fn a_running_command_selects_its_output_without_the_empty_rows_below() {
    let mut tab = tab_with(&["$ make", "building", "", "", ""], &[]);
    tab.cursor.row = 2;
    tab.blocks = vec![Block {
        id: 1,
        prompt: 100,
        command: 100,
        output: Some(101),
        ..Block::default()
    }];
    assert!(tab.select_output(0));
    assert_eq!(tab.selected_text(), Ok(Some("building".into())));
}
