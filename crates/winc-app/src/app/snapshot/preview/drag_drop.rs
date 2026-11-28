//! Markdown 预览中放下块之后的测试：撤销与重做、选区、只读文档、front matter 与收起的标题。

use super::*;
use crate::app::preview::markdown::drag::{Element, element_movable};
use winc_core::Selection;

/// 打开拖动测试文档，并显式打开默认关闭的拖动开关。
fn drag_doc(name: &str, text: &str) -> (App, PathBuf) {
    let (mut app, dir) = live_doc(name, text);
    app.config.markdown_drag = Some(true);
    settle(&mut app);
    (app, dir)
}

/// 把鼠标移到第 `block` 块上，再移到它的把手上并按下，返回把手的横坐标。
fn grab_block(app: &mut App, block: usize) -> f32 {
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let top = app.preview.markdown.block_top(block).unwrap();
    let height = laid.blocks[block].block.height;
    app.on_mouse_move(laid.left + 30.0, top + height / 2.0);
    paint(app);
    let rect = (app.preview.markdown)
        .handle_rect(app, Element::Block(block))
        .expect("应显示把手");
    let (x, y) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    app.on_mouse_move(x, y);
    paint(app);
    app.on_mouse_down(MouseButton::Left);
    x
}

/// 按住第 `block` 块向上拖到第 `target` 块的上沿并松开。
fn drag_block_up(app: &mut App, block: usize, target: usize) {
    let x = grab_block(app, block);
    let top = app.preview.markdown.block_top(target).unwrap();
    app.on_mouse_move(x, top + 1.0);
    app.on_mouse_up(MouseButton::Left);
    settle(app);
}

/// 返回当前文档的全部选区（按位置排序）。
fn selections(app: &App) -> Vec<Selection> {
    app.editor.active().unwrap().selections().0
}

#[test]
fn undo_and_redo_restore_the_text_and_every_cursor() {
    let (mut app, dir) = drag_doc("md-drop-undo", "甲\n\n乙\n\n丙\n");
    // 两个光标：「甲」之后（主光标）与「乙」之后。
    let doc = app.editor.active_mut().unwrap();
    doc.set_selections(vec![Selection::point(1), Selection::point(4)], 0);
    settle(&mut app);
    drag_block_up(&mut app, 2, 0);
    assert_eq!(doc_text(&app), "丙\n\n甲\n\n乙\n");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.selection, Selection::point(4), "主光标随「甲」移动");
    assert_eq!(selections(&app), [Selection::point(4), Selection::point(7)]);

    app.run(winc_editor::Command::Undo);
    settle(&mut app);
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    assert_eq!(app.editor.active().unwrap().selection, Selection::point(1));
    assert_eq!(selections(&app), [Selection::point(1), Selection::point(4)]);
    assert!(!app.editor.active().unwrap().is_dirty());

    app.run(winc_editor::Command::Redo);
    settle(&mut app);
    assert_eq!(doc_text(&app), "丙\n\n甲\n\n乙\n");
    assert_eq!(app.editor.active().unwrap().selection, Selection::point(4));
    assert_eq!(selections(&app), [Selection::point(4), Selection::point(7)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_selection_inside_the_moved_block_moves_with_it() {
    let (mut app, dir) = drag_doc("md-drop-selection", "甲乙\n\n丙\n");
    app.editor.active_mut().unwrap().selection = Selection::new(0, 2);
    settle(&mut app);
    drag_block_up(&mut app, 1, 0);
    assert_eq!(doc_text(&app), "丙\n\n甲乙\n");
    assert_eq!(app.editor.active().unwrap().selection, Selection::new(3, 5));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_on_a_read_only_document_changes_nothing() {
    let (mut app, dir) = drag_doc("md-drop-locked", "甲\n\n乙\n\n丙\n");
    let x = grab_block(&mut app, 2);
    let top = app.preview.markdown.block_top(0).unwrap();
    app.on_mouse_move(x, top + 1.0);
    let revision = app.editor.active().unwrap().revision;
    // 拖动期间文档被设为只读：松手时不改写原文，也不播放放下的过渡。
    app.editor.active_mut().unwrap().set_locked(true);
    assert!(!app.finish_markdown_block_drag());
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    assert_eq!(app.editor.active().unwrap().revision, revision);
    assert!(app.preview.markdown.drag.drop_card.is_none());
    assert!(app.preview.markdown.drag.release.is_none());
    assert!(!app.preview.markdown.drag.slide.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swapping_identical_blocks_leaves_the_document_unmodified() {
    let (mut app, dir) = drag_doc("md-drop-same", "甲\n\n甲\n");
    let revision = app.editor.active().unwrap().revision;
    drag_block_up(&mut app, 1, 0);
    assert_eq!(doc_text(&app), "甲\n\n甲\n");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.revision, revision);
    assert!(!doc.is_dirty());
    assert!(app.preview.markdown.drag.drop_card.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mixed_line_endings_outside_the_moved_block_are_kept() {
    let (mut app, dir) = drag_doc("md-drop-endings", "甲\r\n\r\n乙\n\n丙\n");
    drag_block_up(&mut app, 2, 1);
    let text = doc_text(&app);
    assert!(text.starts_with("甲\r\n\r\n丙"), "{text:?}");
    assert!(text.ends_with("乙\n"), "{text:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn front_matter_stays_at_the_top() {
    let text = "---\ntitle: 测试\n---\n\n甲\n\n乙\n";
    let (mut app, dir) = drag_doc("md-drop-front", text);
    assert!(
        !element_movable(&app, Element::Block(0)),
        "front matter 不可拖动"
    );
    assert!(element_movable(&app, Element::Block(1)));

    // 拖到 front matter 上：不显示插入线，松手后原文不变。
    let x = grab_block(&mut app, 2);
    let top = app.preview.markdown.block_top(0).unwrap();
    app.on_mouse_move(x, top + 1.0);
    let session = app.preview.markdown.drag.session.as_ref().unwrap();
    assert!(session.indicator.is_none());
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), text);

    // 拖到 front matter 之后的第一块之前：照常移动。
    drag_block_up(&mut app, 2, 1);
    assert_eq!(doc_text(&app), "---\ntitle: 测试\n---\n\n乙\n\n甲\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_collapsed_section_is_neither_dragged_nor_dropped_into() {
    let text = "# 一\n\n甲\n\n# 二\n\n乙\n";
    let (mut app, dir) = drag_doc("md-drop-fold", text);
    app.toggle_heading_fold("一");
    settle(&mut app);
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert_eq!(laid.blocks[1].block.height, 0.0, "「甲」应已收起");
    // 收起的标题不可拖动：只移动标题会把收起的一节留在原处。
    assert!(!element_movable(&app, Element::Block(0)));
    assert!(element_movable(&app, Element::Block(2)));

    // 拖到收起的标题与下一个标题之间：放下的块会被收进这一节，不可放置。
    let x = grab_block(&mut app, 3);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(x, top + 1.0);
    let session = app.preview.markdown.drag.session.as_ref().unwrap();
    assert!(session.indicator.is_none());
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), text);

    // 收起的标题之前可以放置。
    drag_block_up(&mut app, 3, 0);
    assert_eq!(doc_text(&app), "乙\n\n# 一\n\n甲\n\n# 二\n");
    let _ = std::fs::remove_dir_all(&dir);
}
