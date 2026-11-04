//! 滚动条自动隐藏的测试：滚动与悬停时显示、切换文档后隐藏，以及 `Shift` 只影响滚轮的水平滚动。

use super::*;
use crate::app::ScrollArea;

#[test]
fn scrollbars_show_while_scrolling_or_hovered() {
    let dir = sample_folder("scrollbar-reveal");
    // 400 行远高于一屏：内容放不下，竖直滚动条有滑块可显示。
    let long: String = (0..400).map(|i| format!("line {i}\n")).collect();
    let path = dir.join("long.txt");
    std::fs::write(&path, long).unwrap();

    let mut app = app_with(&path, false);
    let editor = app.layout.editor;
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    paint(&mut app);

    // 打开文件后没有滚动过，滚动条不显示。
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 0.0);

    // 滚动后显示。
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -200.0,
    )));
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 1.0);

    // 切换到另一个文档不算滚动：新文档的滚动条不显示。
    app.open_path(&dir.join("a.txt"));
    paint(&mut app);
    std::fs::write(dir.join("b.txt"), "x\n".repeat(400)).unwrap();
    app.open_path(&dir.join("b.txt"));
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 0.0);

    // 鼠标移到编辑区右边缘的滚动条位置时显示，移开后先保持显示，随后淡出。
    app.on_mouse_move(editor.right() - 4.0, editor.y + editor.h / 2.0);
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 1.0);
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Editor), 1.0);

    // 内容放得下的文件树没有滑块，鼠标移到文件树的右边缘也不显示。
    let tree = app.layout.tree;
    app.on_mouse_move(tree.right() - 4.0, tree.y + 10.0);
    paint(&mut app);
    assert_eq!(app.scrollbar_shown(ScrollArea::Tree), 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_editor_scrolls_sideways_by_its_scrollbar_and_by_dragging_past_the_edge() {
    let dir = sample_folder("scrollbar-horizontal");
    // 每行 300 格宽：远超编辑区宽度，横向滚动才有可滚的余量。
    let long: String = (0..20)
        .map(|i| format!("line {i} {}\n", "x".repeat(300)))
        .collect();
    let path = dir.join("long.txt");
    std::fs::write(&path, long).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let track = app.text_hscroll_track();
    assert!(app.text_content_w > track.w);

    // 按住滑块拖到右端：滚动到最右边。
    let y = track.bottom() - 4.0;
    app.on_mouse_move(track.x + 8.0, y);
    assert_eq!(app.hover, Hover::Scrollbar(Scrollbar::EditorHorizontal));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(track.right(), y);
    app.on_mouse_up(MouseButton::Left);
    let max = app.text_content_w - track.w;
    assert!((app.editor.active().unwrap().scroll_x - max).abs() < 1.0);

    // 回到左端后拖选文字，鼠标停在右缘外：持续向右滚动，选区随之扩展。
    app.editor.active_mut().unwrap().scroll_x = 0.0;
    paint(&mut app);
    let t = app.layout.text;
    app.on_mouse_move(t.x + 40.0, t.y + 8.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(t.right() + 40.0, t.y + 8.0);
    assert!(app.drag_autoscrolling());
    let first = app.editor.active().unwrap().scroll_x;
    assert!(first > 0.0);
    app.drag_scrolled_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(50));
    app.continue_drag_scroll();
    let doc = app.editor.active().unwrap();
    assert!(doc.scroll_x > first, "{first} → {}", doc.scroll_x);
    let head = doc.selection.head;
    app.drag_scrolled_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(50));
    app.continue_drag_scroll();
    assert!(app.editor.active().unwrap().selection.head > head);

    // 鼠标回到编辑区内时停止滚动。
    app.on_mouse_move(t.x + 40.0, t.y + 8.0);
    assert!(!app.drag_autoscrolling());
    app.on_mouse_up(MouseButton::Left);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn shift_turns_only_mouse_wheel_scrolling_sideways() {
    let dir = sample_folder("scrollbar-shift");
    // 每行 300 格宽：远超编辑区宽度，横向滚动才有可滚的余量。
    let long: String = (0..400)
        .map(|i| format!("    line {i} {}\n", "x".repeat(300)))
        .collect();
    let path = dir.join("long.txt");
    std::fs::write(&path, long).unwrap();

    let mut app = app_with(&path, false);
    let editor = app.layout.editor;
    app.on_mouse_move(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
    app.modifiers = winit::keyboard::ModifiersState::SHIFT;

    // 触控板（含手指离开后的惯性滚动）：按住 `Shift` 仍然竖直滚动，不会横向移动。
    app.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
        0.0, -200.0,
    )));
    let doc = app.editor.active().unwrap();
    assert!(doc.scroll_line > 0.0);
    assert_eq!(doc.scroll_x, 0.0);

    // 鼠标滚轮：按住 `Shift` 时横向滚动。
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -1.0));
    assert!(app.editor.active().unwrap().scroll_x > 0.0);

    std::fs::remove_dir_all(&dir).unwrap();
}
