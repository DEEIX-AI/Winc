//! Markdown 预览中拖动块的测试：悬停显示把手、拖动改顺序、松手后沿用原文。

use super::*;
use crate::app::Drag;
use crate::app::preview::markdown::drag::{Boundary, Element};

/// 打开拖动测试文档，并显式打开默认关闭的拖动开关。
fn drag_doc(name: &str, text: &str) -> (App, PathBuf) {
    let (mut app, dir) = live_doc(name, text);
    app.config.markdown_drag = Some(true);
    settle(&mut app);
    (app, dir)
}

/// 返回第 `block` 块在屏幕上靠左的位置：纵向居中，横向在正文一栏内。
fn block_center(app: &App, block: usize) -> (f32, f32) {
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let area = app.preview.markdown.area;
    let top = area.y - app.preview.markdown.scroll() + laid.pad + laid.tops[block];
    let height = laid.blocks[block].block.height;
    (laid.left + 30.0, top + height / 2.0)
}

/// 返回单元把手中心（窗口坐标）。
fn element_handle(app: &App, element: Element) -> (f32, f32) {
    let rect = app
        .preview
        .markdown
        .handle_rect(app, element)
        .expect("应显示把手");
    (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
}

/// 把鼠标移到单元的把手上并绘制，返回把手中心。
fn hover_element(app: &mut App, element: Element) -> (f32, f32) {
    let (hx, hy) = element_handle(app, element);
    app.on_mouse_move(hx, hy);
    paint(app);
    (hx, hy)
}

/// 把鼠标移到第 `block` 块上并绘制，再移到它的把手上，返回把手中心。
fn hover_block(app: &mut App, block: usize) -> (f32, f32) {
    let (x, y) = block_center(app, block);
    app.on_mouse_move(x, y);
    paint(app);
    hover_element(app, Element::Block(block))
}

#[test]
fn markdown_drag_is_off_by_default_and_enabled_by_the_setting() {
    let (mut app, dir) = live_doc("md-drag-setting", "甲\n\n乙\n");
    assert!(!crate::app::preview::markdown::drag::shows_handles(&app));
    let left_off = app.preview.markdown.layout.as_ref().unwrap().left;
    let (x, y) = block_center(&app, 0);
    app.on_mouse_move(x, y);
    paint(&mut app);
    assert_eq!(app.preview.markdown.drag.hover, None);

    app.settings_set_switch(crate::layout::Setting::MarkdownDrag, true);
    settle(&mut app);
    assert!(crate::app::preview::markdown::drag::shows_handles(&app));
    assert!(
        app.preview.markdown.layout.as_ref().unwrap().left > left_off,
        "打开拖动后应为把手预留左侧空间"
    );
    let (hx, hy) = hover_block(&mut app, 0);
    assert_eq!(app.preview.markdown.drag.hover, Some(Element::Block(0)));
    assert_eq!(app.markdown_handle_at(hx, hy), Some(Element::Block(0)));

    app.settings_set_switch(crate::layout::Setting::MarkdownDrag, false);
    settle(&mut app);
    assert!(!crate::app::preview::markdown::drag::shows_handles(&app));
    assert_eq!(app.preview.markdown.drag.hover, None);
    assert_eq!(
        app.preview.markdown.layout.as_ref().unwrap().left,
        left_off,
        "关闭拖动后不应再预留左侧空间"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hovering_a_block_shows_a_handle_to_its_left() {
    let (mut app, dir) = drag_doc(
        "md-block-handle",
        "# 一级标题\n\n## 二级标题\n\n### 三级标题\n\n正文\n",
    );
    let (x, y) = block_center(&app, 0);
    app.on_mouse_move(x, y);
    paint(&mut app);
    assert_eq!(app.preview.markdown.drag.hover, Some(Element::Block(0)));
    let handle = app
        .preview
        .markdown
        .handle_rect(&app, Element::Block(0))
        .unwrap();
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    // 把手在正文左边，与块的第一行对齐；没有盖住正文。
    assert!(
        handle.right() <= laid.left + 1.0,
        "{handle:?} {:?}",
        laid.left
    );
    assert!(handle.y < y && handle.bottom() > y, "{handle:?}");
    let (hx, hy) = (handle.x + handle.w / 2.0, handle.y + handle.h / 2.0);
    assert_eq!(app.markdown_handle_at(hx, hy), Some(Element::Block(0)));
    // 把手和折叠箭头之间至少留出一整块间距，左移后不能又贴回去。
    let fold = crate::app::preview::markdown::drag::fold_center(
        laid.left,
        app.preview.markdown.drag.numbers > 0.0,
        laid.pad,
    );
    assert!(
        fold - handle.right() >= 12.0,
        "把手 {handle:?} 与折叠箭头 {fold} 太近"
    );
    let numbers = app.preview.markdown.drag.numbers;
    assert_eq!(
        handle.x,
        laid.left
            - crate::app::preview::markdown::drag::gutter(numbers)
            - crate::app::preview::markdown::drag::HANDLE_LEFT_OFFSET
    );
    // 鼠标在正文上时只显示圆点；移到把手上时铺一层底色并显示抓手指针。
    shoot(&mut app, "md-block-handle");
    app.on_mouse_move(hx, hy);
    assert_eq!(
        app.pointer.cursor_icon,
        winit::window::CursorIcon::Grab,
        "鼠标在把手上时显示抓手指针"
    );
    shoot(&mut app, "md-block-handle-active");

    // 移到块外的其他地方：把手消失。
    app.on_mouse_move(app.layout.tree.x + 10.0, 400.0);
    paint(&mut app);
    assert_eq!(app.preview.markdown.drag.hover, None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn moving_within_the_same_slot_still_requests_a_redraw() {
    let (mut app, dir) = drag_doc("md-block-redraw", "甲\n\n乙\n");
    let (hx, y) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    paint(&mut app);
    app.clear_frame_dirty();
    app.on_mouse_move(hx, y + 2.0);
    assert!(app.frame_dirty(), "同一插入目标内移动也要重绘浮层");
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clicking_the_handle_alone_does_not_edit_or_select() {
    let (mut app, dir) = drag_doc("md-block-click", "甲\n\n乙\n\n丙\n");
    let before = doc_text(&app);
    let (hx, hy) = hover_block(&mut app, 0);
    super::click(&mut app, (hx, hy));
    settle(&mut app);
    assert_eq!(doc_text(&app), before);
    assert!(app.editor.active().unwrap().selection.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_the_last_block_up_adds_the_separator() {
    let (mut app, dir) = drag_doc("md-block-drag-up", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 2);
    app.on_mouse_down(MouseButton::Left);
    // 移到第一块的上半部分之前。
    let top = app.preview.markdown.block_top(0).unwrap();
    app.on_mouse_move(hx, top - 6.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Block(0)
    );
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "丙\n\n甲\n\n乙\n\n");
    assert_eq!(app.pointer.drag, Drag::None);
    assert!(app.preview.markdown.drag.session.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_block_between_two_blocks_inserts_it_there() {
    let (mut app, dir) = drag_doc("md-block-drag-middle", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Block(2)
    );
    // 拖动中指示线在目标边界处，原文不变。
    shoot(&mut app, "md-block-drag");
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "乙\n\n甲\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_near_the_bottom_scrolls_the_preview() {
    let text: String = (1..=40).map(|i| format!("第{i}段\n\n")).collect();
    let (mut app, dir) = drag_doc("md-block-scroll", &text);
    let before = app.preview.markdown.scroll();
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let area = app.preview.markdown.area;
    let y = area.bottom() - 4.0;
    for _ in 0..12 {
        app.on_mouse_move(hx, y);
    }
    assert!(
        app.preview.markdown.scroll() > before,
        "拖到预览下边缘时应自动滚动"
    );
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let moved = doc_text(&app);
    assert!(!moved.starts_with("第1段"), "第一段应移到后面：{moved}");
    assert!(moved.find("第1段").is_some_and(|at| at > 0), "{moved}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_table_can_be_dragged_like_any_other_block() {
    let (mut app, dir) = drag_doc(
        "md-block-table",
        "| 名 | 值 |\n|---|---|\n| 甲 | 1 |\n\n后文\n",
    );
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    // 拖到文档末尾：表格移到「后文」之后。
    let area = app.preview.markdown.area;
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let bottom = area.y - app.preview.markdown.scroll() + laid.pad + laid.height + 8.0;
    app.on_mouse_move(hx, bottom.min(area.bottom() - 4.0));
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert!(doc_text(&app).starts_with("后文"));
    assert!(doc_text(&app).contains("| 甲 | 1 |"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_with_a_stale_layout_does_not_rewrite_the_document() {
    let (mut app, dir) = drag_doc("md-drag-stale-layout", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    // 模拟解析已经换了、排版还没跟上的窗口。
    app.preview.markdown.parse_serial = app.preview.markdown.parse_serial.wrapping_add(1);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_code_block_can_be_dragged_to_the_end() {
    let (mut app, dir) = drag_doc(
        "md-code-crash",
        "# 拖动测试\n\n第一段文字，用于拖动。\n\n第二段文字，用于拖动。\n\n```java\npublic static void main() {}\n```\n\n- 列表甲\n- 列表丁\n- 列表丙\n- 列表乙\n- 列表甲\n- 列表丁\n- 列表丙\n- 列表乙\n",
    );
    let (hx, _) = hover_block(&mut app, 3);
    app.on_mouse_down(MouseButton::Left);
    let area = app.preview.markdown.area;
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let bottom = area.y - app.preview.markdown.scroll() + laid.pad + laid.height + 8.0;
    app.on_mouse_move(hx, bottom.min(area.bottom() - 4.0));
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let text = doc_text(&app);
    assert!(text.ends_with("```\n"), "代码块应移到文末：{text:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_list_does_not_swallow_the_blank_line_after_it() {
    for (name, text, expected) in [
        (
            "md-list-after",
            "- 甲\n- 乙\n\n后文\n",
            "- 乙\n- 甲\n\n后文\n",
        ),
        (
            "md-list-after-ordered",
            "1. 甲\n2. 乙\n\n后文\n",
            "1. 乙\n2. 甲\n\n后文\n",
        ),
    ] {
        let (mut app, dir) = drag_doc(name, text);
        let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
        app.on_mouse_down(MouseButton::Left);
        let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 1 });
        app.on_mouse_move(hx, last_y + 16.0);
        app.on_mouse_up(MouseButton::Left);
        settle(&mut app);
        assert_eq!(doc_text(&app), expected);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_downward_block_move_keeps_the_caret_with_the_block() {
    let (mut app, dir) = drag_doc("md-caret-down", "甲\n\n乙\n\n丙\n");
    app.editor.active_mut().unwrap().selection = winc_core::Selection::new(1, 1);
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let area = app.preview.markdown.area;
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let bottom = area.y - app.preview.markdown.scroll() + laid.pad + laid.height + 8.0;
    app.on_mouse_move(hx, bottom.min(area.bottom() - 4.0));
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let text = doc_text(&app);
    assert_eq!(text, "乙\n\n丙\n\n甲\n");
    let caret = app.editor.active().unwrap().selection.head;
    let before: String = text.chars().take(caret).collect();
    assert!(before.ends_with('甲'), "caret {caret} in {text:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_block_keeps_the_viewport_where_the_user_scrolled() {
    let mut text = String::new();
    for i in 0..30 {
        text.push_str(&format!("上方段落 {i}。\n\n"));
    }
    text.push_str(
        "![图](assets/test.png)\n\n**步骤2：自由选择注册或登录方式**\n\n         **步骤3：登录成功**\n\n![尾图](assets/end.png)\n",
    );
    let (mut app, dir) = drag_doc("md-drag-caret-scroll", &text);
    app.editor.active_mut().unwrap().selection = winc_core::Selection::new(0, 0);
    app.start_live_editing();
    app.preview.markdown.scroll_to(app.preview.markdown.height);
    paint(&mut app);
    let before = app.preview.markdown.scroll();
    assert!(before > 300.0, "应先滚到文档下方：{before}");

    let (hx, _) = hover_block(&mut app, 32);
    app.on_mouse_down(MouseButton::Left);
    let (_, second_y) = block_center(&app, 31);
    app.on_mouse_move(hx, second_y);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let after = app.preview.markdown.scroll();
    assert!(
        after > before - 200.0,
        "拖动不应把视口拉回上方光标：before {before}, after {after}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_a_list_item_slides_items_to_new_positions() {
    let (mut app, dir) = drag_doc("md-list-item-motion", "# 标题\n\n- 甲\n- 乙\n- 丙\n");
    app.preview.markdown.frozen = Some(std::time::Duration::from_millis(1));
    let (hx, _) = hover_element(&mut app, Element::Item { block: 1, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 1, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    app.on_mouse_up(MouseButton::Left);
    app.wait_for_markdown();
    paint(&mut app);
    shoot(&mut app, "md-list-item-motion");
    let slide = app
        .preview
        .markdown
        .item_slide(&app, 1)
        .expect("列表项放下后滑动");
    // 各列表项从松手时的视觉位置开始滑动，至少有一项有偏移。
    assert!(
        slide.offsets().iter().any(|offset| offset.abs() > 5.0),
        "{:?}",
        slide.offsets()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_preview_list_items_slide_after_the_new_parse_arrives() {
    let (mut app, dir) = drag_doc("md-list-split-motion", "- 甲\n- 乙\n- 丙\n");
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    app.preview.markdown.frozen = Some(std::time::Duration::from_millis(1));
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    let card = app
        .preview
        .markdown
        .drag
        .drop_card
        .as_ref()
        .expect("分栏解析前应保留松手卡片");
    assert!(card.top > 0.0);
    assert!(
        card.shift.iter().any(|offset| offset.abs() > 0.1),
        "松手时兄弟项仍应保持让位：{:?}",
        card.shift
    );
    app.wait_for_markdown();
    paint(&mut app);
    assert!(app.preview.markdown.drag.drop_card.is_none());
    assert!(
        app.preview.markdown.item_slide(&app, 0).is_some(),
        "分栏等新解析后应开始列表项滑动"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_preview_block_keeps_the_drop_card_before_the_parse() {
    let (mut app, dir) = drag_doc("md-block-split-motion", "甲\n\n乙\n\n丙\n");
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    app.preview.markdown.frozen = Some(std::time::Duration::from_millis(1));
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let area = app.preview.markdown.area;
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let bottom = area.y - app.preview.markdown.scroll() + laid.pad + laid.height + 8.0;
    app.on_mouse_move(hx, bottom.min(area.bottom() - 4.0));
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert!(app.preview.markdown.drag.drop_card.is_some());
    app.wait_for_markdown();
    paint(&mut app);
    assert!(app.preview.markdown.drag.drop_card.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_undo_restores_the_original_order() {
    let (mut app, dir) = drag_doc("md-block-undo", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "乙\n\n甲\n\n丙\n");
    app.run(winc_editor::Command::Undo);
    settle(&mut app);
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_block_slides_the_blocks_between() {
    let (mut app, dir) = drag_doc("md-block-shift", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top + 8.0);
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    shoot(&mut app, "md-block-shift");
    let shift = app
        .preview
        .markdown
        .drag
        .session
        .as_ref()
        .expect("拖动中")
        .shift
        .at(1);
    assert!(shift < -1.0, "乙应向上让位，实际 {shift}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_list_item_slides_the_other_items() {
    let (mut app, dir) = drag_doc("md-list-shift", "- 甲\n- 乙\n- 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    let shift = app
        .preview
        .markdown
        .drag
        .session
        .as_ref()
        .expect("拖动中")
        .shift
        .at(1);
    assert!(shift < -1.0, "乙应向上让位，实际 {shift}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_an_item_up_shifts_the_items_above_it() {
    let (mut app, dir) = drag_doc("md-list-shift-up", "# 标题\n\n- 甲\n- 乙\n- 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 1, item: 2 });
    app.on_mouse_down(MouseButton::Left);
    let (_, first_y) = element_handle(&app, Element::Item { block: 1, item: 0 });
    app.on_mouse_move(hx, first_y - 8.0);
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    let shift = app
        .preview
        .markdown
        .drag
        .session
        .as_ref()
        .expect("拖动中")
        .shift
        .at(0);
    assert!(shift > 1.0, "甲应向下让位，实际 {shift}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_one_of_two_identical_blocks_moves_that_block() {
    let run = |name: &str, text: &str| -> Vec<f32> {
        let (mut app, dir) = drag_doc(name, text);
        app.preview.markdown.frozen = Some(std::time::Duration::from_millis(1));
        let (hx, _) = hover_block(&mut app, 1);
        app.on_mouse_down(MouseButton::Left);
        let top = app.preview.markdown.block_top(0).unwrap();
        app.on_mouse_move(hx, top - 8.0);
        app.on_mouse_up(MouseButton::Left);
        app.wait_for_markdown();
        paint(&mut app);
        let from = app
            .preview
            .markdown
            .motion
            .as_ref()
            .expect("重排后滑动")
            .from
            .clone();
        let _ = std::fs::remove_dir_all(&dir);
        from
    };
    // 相同的两块与不同的两块，几何相同；拖动结束时的视觉偏移也应相同。
    let duplicate = run("md-block-duplicate", "相同\n\n相同\n\n乙\n");
    let distinct = run("md-block-distinct", "相同\n\n丙\n\n乙\n");
    assert_eq!(duplicate.len(), distinct.len());
    for (a, b) in duplicate.iter().zip(&distinct) {
        assert!((a - b).abs() < 0.5, "{duplicate:?} vs {distinct:?}");
    }
}

#[test]
fn dragging_an_item_up_over_another_makes_it_move_before_release() {
    let (mut app, dir) = drag_doc("md-list-swap-up", "- 丁\n- 甲\n- 丙\n- 乙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 2 });
    app.on_mouse_down(MouseButton::Left);
    let (_, second_y) = element_handle(&app, Element::Item { block: 0, item: 1 });
    app.on_mouse_move(hx, second_y);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Item(1)
    );
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    let shift = app
        .preview
        .markdown
        .drag
        .session
        .as_ref()
        .unwrap()
        .shift
        .at(1);
    assert!(shift > 1.0, "甲应在松手前向下让位，实际 {shift}");
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_block_over_another_makes_it_move_before_release() {
    let (mut app, dir) = drag_doc("md-block-swap", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let (_, second_y) = block_center(&app, 1);
    app.on_mouse_move(hx, second_y);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Block(2)
    );
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    let shift = app
        .preview
        .markdown
        .drag
        .session
        .as_ref()
        .unwrap()
        .shift
        .at(1);
    assert!(shift < -1.0, "乙应在松手前向上让位，实际 {shift}");
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_list_item_over_another_makes_it_move_before_release() {
    let (mut app, dir) = drag_doc("md-list-swap", "- 甲\n- 乙\n- 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, second_y) = element_handle(&app, Element::Item { block: 0, item: 1 });
    app.on_mouse_move(hx, second_y + 12.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Item(2)
    );
    paint(&mut app);
    paint(&mut app);
    paint(&mut app);
    let shift = app
        .preview
        .markdown
        .drag
        .session
        .as_ref()
        .unwrap()
        .shift
        .at(1);
    assert!(shift < -1.0, "乙应在松手前向上让位，实际 {shift}");
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_a_block_slides_each_block_to_its_new_place() {
    let (mut app, dir) = drag_doc("md-block-drag-motion", "甲\n\n乙\n\n丙\n");
    // 固定动画进度：抬起和重排都停在开头，能看清起始偏移。
    app.preview.markdown.frozen = Some(std::time::Duration::from_millis(1));
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    app.on_mouse_up(MouseButton::Left);
    app.wait_for_markdown();
    paint(&mut app);
    let motion = app.preview.markdown.motion.as_ref().expect("重排后滑动");
    assert!(motion.moving());
    // 至少一块从旧位置滑到新位置；松手时的让位偏移也并入了起点。
    assert!(
        motion.from.iter().any(|offset| offset.abs() > 5.0),
        "{:?}",
        motion.from
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_handle_keeps_clear_of_two_digit_line_numbers() {
    let mut text = String::new();
    for i in 0..5 {
        text.push_str(&format!("第 {i} 段文字。\n\n"));
    }
    text.push_str("# 使用说明\n");
    let (mut app, dir) = drag_doc("md-handle-two-digit-lines", &text);
    app.config.markdown_line_numbers = Some(true);
    settle(&mut app);
    let (_, _) = hover_block(&mut app, 5);
    let handle = app
        .preview
        .markdown
        .handle_rect(&app, Element::Block(5))
        .expect("标题应显示把手");
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let numbers = app.preview.markdown.drag.numbers;
    assert!(numbers > 0.0, "应显示行号");
    // 行号栏的文字左边缘 = 正文左边 - 行号栏总宽；把手右缘还要再留出至少 12px。
    assert!(
        handle.right() <= laid.left - numbers - 12.0,
        "把手 {handle:?} 压到了行号栏（正文 {:?}，行号栏宽 {numbers}）",
        laid.left
    );
    shoot(&mut app, "md-handle-two-digit-lines");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn handles_still_work_without_line_numbers() {
    let (mut app, dir) = drag_doc("md-block-no-numbers", "甲\n\n乙\n\n丙\n");
    app.config.markdown_line_numbers = Some(false);
    settle(&mut app);
    let (hx, _) = hover_block(&mut app, 0);
    let handle = app
        .preview
        .markdown
        .handle_rect(&app, Element::Block(0))
        .unwrap();
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    assert_eq!(
        handle.x,
        laid.left
            - crate::app::preview::markdown::drag::gutter(0.0)
            - crate::app::preview::markdown::drag::HANDLE_LEFT_OFFSET
    );
    let fold = crate::app::preview::markdown::drag::fold_center(laid.left, false, laid.pad);
    assert!(
        fold - handle.right() >= 12.0,
        "无行号时把手 {handle:?} 与折叠箭头 {fold} 太近"
    );
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "乙\n\n甲\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_insert_line_hides_when_the_target_is_a_no_op() {
    let (mut app, dir) = drag_doc("md-block-noop-line", "甲\n\n乙\n\n丙\n");
    let (hx, _) = hover_block(&mut app, 0);
    app.on_mouse_down(MouseButton::Left);
    let (_, target_y) = {
        let top = app.preview.markdown.block_top(2).unwrap();
        (hx, top - 4.0)
    };
    app.on_mouse_move(hx, target_y);
    assert!(
        app.preview
            .markdown
            .drag
            .session
            .as_ref()
            .unwrap()
            .indicator
            .is_some(),
        "能改变顺序时应显示指示线"
    );
    // 拖回原位：目标不再改变顺序，指示线应消失。
    let top = app.preview.markdown.block_top(0).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    assert!(
        app.preview
            .markdown
            .drag
            .session
            .as_ref()
            .unwrap()
            .indicator
            .is_none(),
        "原位不应承诺一次移动"
    );
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_handle_does_not_flicker_between_paragraphs() {
    let (mut app, dir) = drag_doc("md-block-gap", "甲\n\n乙\n\n丙\n");
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let area = app.preview.markdown.area;
    let gap = laid.tops[1] - (laid.tops[0] + laid.blocks[0].block.height);
    assert!(gap > 2.0, "两段之间应有间距：{gap}");
    let y = area.y - app.preview.markdown.scroll()
        + laid.pad
        + laid.tops[0]
        + laid.blocks[0].block.height
        + gap / 2.0;
    let x = laid.left + 20.0;
    app.on_mouse_move(x, y);
    paint(&mut app);
    assert_eq!(
        app.preview.markdown.drag.hover,
        Some(Element::Block(0)),
        "鼠标在段落间距里时把手仍属于上一段"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_a_block_back_where_it_was_changes_nothing() {
    let (mut app, dir) = drag_doc("md-block-drag-same", "甲\n\n乙\n\n丙\n");
    let (hx, hy) = hover_block(&mut app, 1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(hx, hy);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "甲\n\n乙\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_handle_fits_in_a_narrow_window() {
    let (mut app, dir) = drag_doc(
        "md-block-narrow",
        "# 一级标题\n\n## 二级\n\n### 三级标题\n\n正文\n",
    );
    // 与参考截图相近的窗口大小：没有侧栏，正文宽度不足 880，左侧留白只有几十像素。
    app.toggle_sidebar();
    app.frame.renderer =
        Some(cpui_render::Renderer::offscreen(660.0, 414.0, SCALE, app.palette.frame_bg).unwrap());
    app.relayout();
    settle(&mut app);
    let (hx, hy) = hover_block(&mut app, 0);
    let handle = app
        .preview
        .markdown
        .handle_rect(&app, Element::Block(0))
        .unwrap();
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let area = app.preview.markdown.area;
    // 把手完整落在预览区域内，并且在正文左边。
    assert!(handle.x >= area.x, "{handle:?} {area:?}");
    assert!(
        handle.right() <= laid.left + 1.0,
        "{handle:?} {:?}",
        laid.left
    );
    assert_eq!(app.markdown_handle_at(hx, hy), Some(Element::Block(0)));
    shoot(&mut app, "md-block-handle-narrow");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_split_preview_drags_from_the_right_pane_only() {
    let (mut app, dir) = drag_doc("md-block-split", "甲\n\n乙\n\n丙\n");
    app.set_preview_mode(PreviewMode::Split);
    settle(&mut app);
    assert_eq!(app.preview.markdown.area, app.layout.split);
    // 右栏悬停块显示把手。
    let (hx, _) = hover_block(&mut app, 0);
    assert_eq!(app.preview.markdown.drag.hover, Some(Element::Block(0)));
    assert!(
        app.preview
            .markdown
            .handle_rect(&app, Element::Block(0))
            .is_some()
    );
    // 左栏（源代码）上同样的纵向位置没有把手。
    let left = app.layout.text;
    let laid = app.preview.markdown.layout.as_ref().unwrap();
    let top = app.preview.markdown.area.y - app.preview.markdown.scroll() + laid.pad + laid.tops[0];
    assert_eq!(app.markdown_handle_at(left.x + 30.0, top + 6.0), None);
    // 在右栏把手处拖动：原文重排仍然生效。
    app.on_mouse_down(MouseButton::Left);
    let top = app.preview.markdown.block_top(2).unwrap();
    app.on_mouse_move(hx, top - 4.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "乙\n\n甲\n\n丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_list_item_can_be_dragged_within_its_list() {
    let (mut app, dir) = drag_doc("md-list-item", "- 甲\n- 乙\n- 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    assert_eq!(
        app.preview.markdown.drag.hover,
        Some(Element::Item { block: 0, item: 0 })
    );
    app.on_mouse_down(MouseButton::Left);
    // 移到第三项上：向下拖越过它的上边界，插到它后面。
    let (_, third_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, third_y - 2.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Item(3)
    );
    shoot(&mut app, "md-list-item-drag");
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "- 乙\n- 丙\n- 甲\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_nested_list_item_moves_with_its_children() {
    let (mut app, dir) = drag_doc("md-list-nested", "- 甲\n  - 甲一\n  - 甲二\n- 乙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 1 });
    app.on_mouse_move(hx, last_y + 16.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "- 乙\n- 甲\n  - 甲一\n  - 甲二\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_list_item_can_move_to_the_end_and_undo() {
    let (mut app, dir) = drag_doc("md-list-item-end", "- 甲\n- 乙\n- 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Item(3)
    );
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "- 乙\n- 丙\n- 甲\n");
    app.run(winc_editor::Command::Undo);
    settle(&mut app);
    assert_eq!(doc_text(&app), "- 甲\n- 乙\n- 丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ordered_list_items_move_and_are_renumbered() {
    let (mut app, dir) = drag_doc("md-list-ordered", "1. 甲\n2. 乙\n3. 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Item(3)
    );
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "1. 乙\n2. 丙\n3. 甲\n");
    // 整个列表一次替换，撤销一次应恢复原顺序和原编号。
    app.run(winc_editor::Command::Undo);
    settle(&mut app);
    assert_eq!(doc_text(&app), "1. 甲\n2. 乙\n3. 丙\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ordered_lists_keep_their_start_and_delimiter() {
    let (mut app, dir) = drag_doc("md-list-ordered-start", "5) 甲\n6) 乙\n7) 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "5) 乙\n6) 丙\n7) 甲\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ordered_items_growing_to_two_digits_keep_their_continuation_indent() {
    let text: String = (1..=10)
        .map(|i| format!("1. 第{i}项\n   续{i}\n"))
        .collect();
    let (mut app, dir) = drag_doc("md-list-ordered-grow", &text);
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 9 });
    app.on_mouse_move(hx, last_y + 20.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    let text = doc_text(&app);
    assert!(text.starts_with("1. 第2项\n   续2\n"), "{text:?}");
    assert!(text.contains("10. 第1项\n    续1\n"), "{text:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ordered_task_items_keep_their_checkboxes() {
    let (mut app, dir) = drag_doc("md-list-ordered-tasks", "1. [ ] 甲\n2. [x] 乙\n3. [ ] 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "1. [x] 乙\n2. [ ] 丙\n3. [ ] 甲\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dragging_an_ordered_item_across_marker_widths() {
    let text = "1. 甲\n   续甲\n10. 乙\n    续乙\n11. 丙\n    续丙\n";
    let (mut app, dir) = drag_doc("md-list-ordered-widths", text);
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 20.0);
    assert_eq!(
        app.preview.markdown.drag.session.as_ref().unwrap().target,
        Boundary::Item(3)
    );
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(
        doc_text(&app),
        "1. 乙\n   续乙\n2. 丙\n   续丙\n3. 甲\n   续甲\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn task_list_items_move_with_their_checkboxes() {
    let (mut app, dir) = drag_doc("md-list-tasks", "- [ ] 甲\n- [x] 乙\n- [ ] 丙\n");
    let (hx, _) = hover_element(&mut app, Element::Item { block: 0, item: 0 });
    app.on_mouse_down(MouseButton::Left);
    let (_, last_y) = element_handle(&app, Element::Item { block: 0, item: 2 });
    app.on_mouse_move(hx, last_y + 16.0);
    app.on_mouse_up(MouseButton::Left);
    settle(&mut app);
    assert_eq!(doc_text(&app), "- [x] 乙\n- [ ] 丙\n- [ ] 甲\n");
    let _ = std::fs::remove_dir_all(&dir);
}
