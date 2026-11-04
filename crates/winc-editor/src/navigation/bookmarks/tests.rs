//! 书签的测试：切换与回绕、书签随编辑移动或删除、按行号排序去重。

use super::*;
use crate::DocumentId;
use winc_core::Selection;
use winc_core::edit::{EditKind, Transaction};

fn doc(text: &str) -> Document {
    Document::from_text(DocumentId(1), text)
}

fn insert(doc: &mut Document, at: usize, text: &str) {
    let change = winc_core::edit::Change {
        start: at,
        removed: String::new(),
        inserted: text.into(),
    };
    let after = Selection::point(at + text.chars().count());
    doc.apply(Transaction::new(
        vec![change],
        doc.selection,
        after,
        EditKind::Other,
    ));
}

fn remove(doc: &mut Document, from: usize, to: usize) {
    let change = winc_core::edit::Change {
        start: from,
        removed: doc.text().text_range(from, to, 1 << 20).unwrap_or_default(),
        inserted: String::new(),
    };
    doc.apply(Transaction::new(
        vec![change],
        Selection::point(from),
        Selection::point(from),
        EditKind::Other,
    ));
}

#[test]
fn toggling_and_moving_between_them() {
    let mut marks = Bookmarks::default();
    assert!(marks.toggle(2));
    assert!(marks.toggle(0));
    assert!(!marks.toggle(2));
    assert_eq!(marks.lines(), [0]);
    marks.toggle(5);
    assert_eq!(marks.lines(), [0, 5]);

    // 向后查找越过最后一个书签时回绕到第一个，向前查找越过第一个书签时回绕到最后一个。
    assert_eq!(marks.next(0), Some(5));
    assert_eq!(marks.next(5), Some(0));
    assert_eq!(marks.next(4), Some(5));
    assert_eq!(marks.previous(5), Some(0));
    assert_eq!(marks.previous(0), Some(5));
    assert_eq!(Bookmarks::default().next(3), None);

    marks.clamp(5);
    assert_eq!(marks.lines(), [0]);
}

#[test]
fn bookmarks_follow_the_lines_they_are_on() {
    let mut document = doc("a\nb\nc\nd\ne\n");
    document.selection = Selection::point(document.text().line_start(2));
    assert!(document.toggle_bookmark());
    assert_eq!(document.bookmark_lines(), [2]);

    // 在书签所在行之前插入一行，书签下移一行。
    insert(&mut document, 0, "zero\n");
    assert_eq!(document.bookmark_lines(), [3]);

    // 在书签所在行的中间输入，书签仍在该行。
    let at = document.text().line_start(3) + 1;
    insert(&mut document, at, "x");
    assert_eq!(document.bookmark_lines(), [3]);
    assert_eq!(document.bookmark_preview(3), "cx");

    // 删除书签之前的两行，书签上移两行。
    let at = document.text().line_start(2);
    remove(&mut document, 0, at);
    assert_eq!(document.bookmark_lines(), [1]);

    // 整行删除书签所在的行（包括换行符），书签一并删除。
    let line = document.text().line_start(1);
    let next = document.text().line_start(2);
    remove(&mut document, line, next);

    assert!(document.bookmark_lines().is_empty());

    // 撤销这次删除不会恢复已删除的书签；撤销之前新加的书签位于插入点所在行，仍是第 1 行。
    document.toggle_bookmark();
    document.execute(crate::Command::Undo, 10, 4);
    assert_eq!(document.bookmark_lines(), [1]);

    // 删除书签所在行之前的一行（⇧⌘K，删除范围止于下一行的行首）：书签所在行保留，书签上移一行。
    assert_eq!(document.bookmark_lines(), [1]);
    document.selection = Selection::point(document.text().line_start(0));
    document.execute(crate::Command::DeleteLine, 10, 4);
    assert_eq!(
        document.bookmark_lines(),
        [0],
        "书签那一行上来了，书签跟着挪"
    );
}

#[test]
fn saved_lines_are_sorted_once() {
    assert_eq!(Bookmarks::from_lines(vec![3, 0, 3, 1]).lines(), [0, 1, 3]);
}
