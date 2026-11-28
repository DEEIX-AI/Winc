//! 分栏预览的位置、鼠标交互与设置写回。

use super::*;
use crate::app::input::context_menu::MenuCommand;
use crate::layout::SettingControl;
use crate::theme::{SCROLLBAR_HIT, TEXT_PAD};

#[test]
fn left_split_preview_moves_source_geometry_and_preserves_mouse_input() {
    let (mut app, dir) = readme("md-left-split", false, PreviewMode::Split);
    app.config.minimap = Some(true);
    app.open_search(false);
    app.relayout();
    let default = app.layout;
    assert_eq!(default.split.x, default.editor.right() + 1.0);
    assert!(default.minimap.w > 0.0);

    app.config.split_preview_left = Some(true);
    app.relayout();
    paint(&mut app);
    let left = app.layout;
    assert_eq!(left.split.x, default.editor.x);
    assert_eq!(left.split.w, default.editor.w);
    assert_eq!(left.editor.x, left.split.right() + 1.0);
    assert_eq!(left.editor.right(), default.split.right());
    assert_eq!(left.gutter.x, left.editor.x);
    assert_eq!(left.text.x, left.gutter.right());
    assert_eq!(left.minimap.x, left.text.right());
    assert_eq!(left.minimap.right(), left.editor.right() - SCROLLBAR_HIT);
    assert!(left.search.x >= left.editor.x && left.search.right() <= left.editor.right());
    assert_eq!(left.split_divider, default.split_divider);
    assert_eq!(app.preview.markdown.area, left.split);

    click(&mut app, (left.split.x + 5.0, left.split.bottom() - 5.0));
    assert!(app.markdown_focused());
    let x = left.text.x + TEXT_PAD + 4.0 * app.metrics.cell;
    let y = left.search.bottom() + app.fonts.line_height;
    let expected = app.pos_at(x, y).unwrap();
    assert!(expected > 0);
    click(&mut app, (x, y));
    assert!(!app.markdown_focused());
    assert_eq!(
        app.editor.active().unwrap().selection,
        winc_core::Selection::point(expected)
    );

    let divider_x = |app: &App| {
        let rect = app.layout.split_divider;
        rect.x + rect.w / 2.0
    };
    let x = divider_x(&app);
    let y = left.split_divider.y + left.split_divider.h / 2.0;
    app.on_mouse_move(x, y);
    assert_eq!(app.pointer.hover, Hover::SplitDivider);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 100.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(divider_x(&app), x + 100.0);
    assert_eq!(app.layout.split.w, left.split.w + 100.0);
    assert!(app.layout.search.x >= app.layout.editor.x);
    assert!(app.layout.search.right() <= app.layout.editor.right());

    for _ in 0..2 {
        click(&mut app, (x + 100.0, y));
    }
    assert_eq!(app.preview.split_share, None);
    assert_eq!(app.layout.split_divider, default.split_divider);

    paint(&mut app);
    app.on_mouse_move(left.split.x + 5.0, y);
    app.on_wheel(MouseScrollDelta::PixelDelta((0.0, -500.0).into()));
    let preview_scroll = app.preview.markdown.scroll();
    assert!(preview_scroll > 0.0);
    assert!(app.editor.active().unwrap().scroll_line > 0.0);
    app.on_mouse_move(left.text.x + 5.0, y);
    app.on_wheel(MouseScrollDelta::PixelDelta((0.0, 500.0).into()));
    paint(&mut app);
    assert_eq!(app.editor.active().unwrap().scroll_line, 0.0);
    assert!(app.preview.markdown.scroll() < preview_scroll);

    app.config.split_preview_left = Some(false);
    app.relayout();
    let restored = app.layout;
    assert_eq!(restored.editor, default.editor);
    assert_eq!(restored.gutter, default.gutter);
    assert_eq!(restored.text, default.text);
    assert_eq!(restored.minimap, default.minimap);
    assert_eq!(restored.search, default.search);
    assert_eq!(restored.split, default.split);
    assert_eq!(restored.split_divider, default.split_divider);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn markdown_settings_save_split_preview_position_and_relayout_immediately() {
    let (mut app, dir) = readme("md-split-position-setting", false, PreviewMode::Split);
    let config_path = dir.join("settings.json");
    app.store.config_path = Some(config_path.clone());
    app.open_settings();
    app.settings_show_page(SettingsPage::Markdown);
    paint(&mut app);

    let setting = Setting::SplitPreviewPosition;
    assert_eq!(setting.control(), SettingControl::Choice);
    let right = MenuCommand::SplitPreviewPosition(false);
    let left = MenuCommand::SplitPreviewPosition(true);
    for (before, selected, preview_left) in [(right, left, true), (left, right, false)] {
        app.settings_open_menu(setting);
        assert_eq!(menu_commands(&app), [(right, true), (left, true)]);
        assert!(app.menu_checked(before));
        assert!(!app.menu_checked(selected));
        app.run_menu_command(selected);
        assert!(app.menu_checked(selected));
        assert_eq!(app.config.split_preview_left, Some(preview_left));
        let (saved, problems) = Config::load_from(&config_path);
        assert!(problems.is_empty());
        assert_eq!(saved.split_preview_left, Some(preview_left));
        assert_eq!(app.layout.split.x < app.layout.editor.x, preview_left);
    }
    app.settings.search.set_text(setting.label());
    app.settings_search_changed();
    assert!(app.settings_layout().row(setting).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}
