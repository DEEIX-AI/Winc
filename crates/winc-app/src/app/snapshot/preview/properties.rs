//! Properties 预览的测试：三段切换、分栏实时显示、只读预览、滚动和模式记忆。

use winc_core::Lines;
use winc_editor::{Command, PreviewKind, PreviewMode};

use super::*;

/// 文件内容：可读的转义、已经是字符的文字、注释、空行和重复的键。
const SOURCE: &str = "greeting=\\u4F60\\u597D\nfarewell=\\u518D\\u89C1 # \\u6CE8\\u91CA\n\nkey=\\u4E00\nkey=\\u4E8C\nemoji=\\uD83D\\uDE00\nliteral=\\\\u4F60\n";

/// 打开写有 [`SOURCE`] 的 `.properties` 文件，等界面画好。
fn properties(name: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("messages_zh.properties");
    std::fs::write(&path, SOURCE).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    (app, dir)
}

/// 点击标签栏上的显示方式，使用与其他预览相同的三段切换。
fn click_mode(app: &mut App, mode: PreviewMode) {
    let [source, split, preview] = crate::layout::preview_segments(app.layout.preview);
    let button = match mode {
        PreviewMode::Source => source,
        PreviewMode::Split => split,
        PreviewMode::Preview => preview,
    };
    click(app, (button.x + button.w / 2.0, button.y + button.h / 2.0));
    paint(app);
}

/// 取离屏帧中一行开头的像素。避开窗格边缘、滚动条和顶栏。
fn row_pixels(app: &App, pane: Rect, row: usize) -> Vec<u32> {
    let renderer = app.frame.renderer.as_ref().unwrap();
    let image = renderer.image().unwrap();
    let scale = renderer.scale_factor();
    let left = ((pane.x + 8.0) * scale).ceil() as usize;
    let right = ((pane.x + pane.w.min(220.0)) * scale) as usize;
    let top = ((pane.y + row as f32 * app.fonts.line_height) * scale).ceil() as usize;
    let bottom = top + (app.fonts.line_height * scale) as usize;
    let stride = image.width as usize;
    (top..bottom)
        .flat_map(|y| &image.pixels[y * stride + left..y * stride + right])
        .copied()
        .collect()
}

/// 通过平台的标准复制或全选快捷键执行操作。
fn preview_shortcut(app: &mut App, key: char) -> bool {
    use crate::keymap::{KeyCode, Mods, Stroke};
    let mods = if cfg!(target_os = "macos") {
        Mods::META
    } else {
        Mods::CTRL
    };
    app.on_shortcut(Stroke {
        key: KeyCode::Char(key),
        mods,
    })
}

/// 预览中物理行与显示列交点，使用与文字绘制相同的字体尺寸。
fn preview_point(app: &App, row: usize, cell: usize) -> (f32, f32) {
    let pane = app.properties_area().unwrap();
    (
        pane.x + crate::theme::TEXT_PAD + cell as f32 * app.metrics.cell,
        pane.y + (row as f32 + 0.5) * app.fonts.line_height,
    )
}

#[test]
fn properties_preview_selects_and_copies_decoded_text_with_shortcuts() {
    use crate::keymap::Action;
    use_vscode_keys();

    let expected =
        "greeting=你好\nfarewell=再见 # 注释\n\nkey=一\nkey=二\nemoji=😀\nliteral=\\\\u4F60\n";
    for mode in [PreviewMode::Preview, PreviewMode::Split] {
        let (mut app, dir) = properties(&format!("properties-copy-{mode:?}"));
        let original = app.editor.active().unwrap().selection;
        click_mode(&mut app, mode);
        let origin = preview_point(&app, 0, 0);
        click(&mut app, origin);
        app.clipboard.copied = Some("unchanged".into());
        assert!(preview_shortcut(&mut app, 'c'));
        assert_eq!(app.clipboard.copied.as_deref(), Some("unchanged"));
        let revision = app.editor.active().unwrap().revision;
        let pane = app.properties_area().unwrap();
        let unselected = row_pixels(&app, pane, 0);

        assert!(preview_shortcut(&mut app, 'a'));
        paint(&mut app);
        assert_ne!(unselected, row_pixels(&app, pane, 0));
        assert!(preview_shortcut(&mut app, 'c'));
        assert_eq!(app.clipboard.copied.as_deref(), Some(expected));
        assert!(preview_shortcut(&mut app, 'x'));
        assert_eq!(app.clipboard.copied.as_deref(), Some(expected));
        assert!(app.run_action(Action::ToggleVimMode));
        assert!(preview_shortcut(&mut app, 'c'));
        assert_eq!(app.clipboard.copied.as_deref(), Some(expected));
        assert!(!app.action_available(Action::Paste));
        app.run(Command::Insert("unexpected".into()));

        let doc = app.editor.active().unwrap();
        assert_eq!(doc.revision, revision);
        assert_eq!(doc.selection, original);
        assert!(!doc.is_dirty());
        assert_eq!(doc.text().rope().unwrap().to_string(), SOURCE);
        assert_eq!(
            std::fs::read_to_string(dir.join("messages_zh.properties")).unwrap(),
            SOURCE
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn properties_preview_drag_selection_and_context_copy_use_display_columns() {
    use crate::app::input::context_menu::MenuCommand;

    let (mut app, dir) = properties("properties-drag-copy");
    click_mode(&mut app, PreviewMode::Split);
    let start = preview_point(&app, 0, 9);
    let end = preview_point(&app, 1, 11);
    app.on_mouse_move(start.0, start.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(end.0, end.1);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);

    let inside = preview_point(&app, 0, 10);
    right_click(&mut app, inside);
    let commands = menu_commands(&app);
    assert!(commands.contains(&(MenuCommand::Copy, true)));
    assert!(commands.contains(&(MenuCommand::SelectAll, true)));
    app.run_menu_command(MenuCommand::Copy);
    assert_eq!(app.clipboard.copied.as_deref(), Some("你好\nfarewell=再"));
    right_click(&mut app, inside);
    app.run_menu_command(MenuCommand::SelectAll);
    right_click(&mut app, inside);
    app.run_menu_command(MenuCommand::Copy);
    assert!(app.clipboard.copied.as_ref().unwrap().contains("emoji=😀"));
    assert_eq!(
        app.editor
            .active()
            .unwrap()
            .text()
            .rope()
            .unwrap()
            .to_string(),
        SOURCE
    );
    click_mode(&mut app, PreviewMode::Preview);
    app.notify(NoticeKind::Error, "Test notice");
    paint(&mut app);
    let notice = app.hits.notice_card.unwrap();
    assert!(app.properties_area_at(notice.x + notice.w / 2.0, notice.y + notice.h / 2.0));
    assert!(
        app.properties_context_target(notice.x + notice.w / 2.0, notice.y + notice.h / 2.0)
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn properties_preview_copy_all_includes_text_beyond_the_render_limit() {
    use_vscode_keys();
    let dir = sample_folder("properties-copy-long-line");
    let path = dir.join("long.properties");
    let source = format!("key={}\nnext=\\u597D\n", "\\u4F60".repeat(12_000));
    std::fs::write(&path, &source).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    click_mode(&mut app, PreviewMode::Preview);
    assert!(preview_shortcut(&mut app, 'a'));
    assert!(preview_shortcut(&mut app, 'c'));
    assert_eq!(
        app.clipboard.copied,
        Some(format!("key={}\nnext=好\n", "你".repeat(12_000)))
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn properties_preview_selection_handles_tabs_wide_text_and_source_changes() {
    use crate::keymap::Action;
    use_vscode_keys();
    let dir = sample_folder("properties-copy-columns");
    let path = dir.join("columns.properties");
    let source = "value=\t\\u4F60\\u597D\\uD83D\\uDE00\r\n";
    std::fs::write(&path, source).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    click_mode(&mut app, PreviewMode::Split);
    let start = preview_point(&app, 0, 8);
    let end = preview_point(&app, 0, 14);
    app.on_mouse_move(start.0, start.1);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(end.0, end.1);
    app.on_mouse_up(MouseButton::Left);
    assert!(preview_shortcut(&mut app, 'c'));
    assert_eq!(app.clipboard.copied.as_deref(), Some("你好😀"));
    assert!(preview_shortcut(&mut app, 'a'));
    assert!(preview_shortcut(&mut app, 'c'));
    assert_eq!(app.clipboard.copied.as_deref(), Some("value=\t你好😀\r\n"));

    // 回到源码一侧修改，旧选区在下一次复制前作废。
    let text = app.layout.text;
    let source_at = (text.x + 10.0, text.y + app.fonts.line_height / 2.0);
    click(&mut app, source_at);
    assert!(app.run_action(Action::SelectAll));
    app.run(Command::Insert("value=\\u4E2D\n".into()));
    assert!(app.properties_selected_text().is_none());
    paint(&mut app);
    let reader = preview_point(&app, 0, 0);
    click(&mut app, reader);
    assert!(preview_shortcut(&mut app, 'a'));
    assert!(preview_shortcut(&mut app, 'c'));
    assert_eq!(app.clipboard.copied.as_deref(), Some("value=中\r\n"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);

    click_mode(&mut app, PreviewMode::Source);
    assert!(app.run_action(Action::Undo));
    assert_eq!(
        app.editor
            .active()
            .unwrap()
            .text()
            .rope()
            .unwrap()
            .to_string(),
        source
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 预览显示转义对应的字符，仍是只读的；切回代码后文字、可编辑性和撤销记录都还在，磁盘上的
/// 文件没有变化。
#[test]
fn properties_preview_decodes_escapes_and_keeps_the_source() {
    let (mut app, dir) = properties("properties-preview");
    let path = dir.join("messages_zh.properties");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_kind(), Some(PreviewKind::Properties));
    assert_eq!(doc.preview_mode(), PreviewMode::Source);
    assert_eq!(app.layout.preview.w, crate::layout::PREVIEW_GROUP_W);

    // 在分栏中改动源码，右侧实际画出的第二行随未保存的修改改变。
    click_mode(&mut app, PreviewMode::Split);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Split);
    assert!(doc.shows_text() && !doc.readonly);
    assert!(app.layout.split.w > 0.0);
    let before = row_pixels(&app, app.layout.split, 1);
    app.run_action(crate::keymap::Action::SelectAll);
    app.run(Command::Insert(
        "greeting=\\u4F60\\u597D\nkey=\\u4E2D\n".into(),
    ));
    paint(&mut app);
    assert!(app.editor.active().unwrap().is_dirty());
    assert_ne!(before, row_pixels(&app, app.layout.split, 1));
    let pane = app.layout.split;
    click(&mut app, (pane.x + pane.w / 2.0, pane.y + pane.h / 2.0));
    assert!(!app.action_available(crate::keymap::Action::Paste));

    // 转义和直接写入的中文在预览中画出相同的字形。
    app.open_search(false);
    assert!(app.bars.find.open);
    click_mode(&mut app, PreviewMode::Preview);
    assert!(!app.bars.find.open);
    assert!(!app.action_available(crate::keymap::Action::FindNext));
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.preview_mode(), PreviewMode::Preview);
    assert!(doc.readonly && !doc.shows_text());
    let reference = dir.join("decoded.properties");
    std::fs::write(&reference, "greeting=你好\nkey=中\n").unwrap();
    let mut decoded = app_with(&reference, false);
    paint(&mut decoded);
    decoded.set_preview_mode(PreviewMode::Preview);
    paint(&mut decoded);
    for row in 0..2 {
        let rendered = row_pixels(&app, app.layout.editor, row);
        assert!(rendered.iter().any(|pixel| *pixel != rendered[0]));
        assert_eq!(rendered, row_pixels(&decoded, decoded.layout.editor, row));
    }

    // 预览是只读的：输入不改变文档（版本号不变；先前的修改仍未保存）。
    let revision = app.editor.active().unwrap().revision;
    app.run(Command::Insert("x".into()));
    assert_eq!(app.editor.active().unwrap().revision, revision);
    // 预览期间磁盘上的文件仍是原文，未保存的修改没有写出去。
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SOURCE);

    // 切回代码恢复可编辑，未保存的修改仍在。
    click_mode(&mut app, PreviewMode::Source);
    let doc = app.editor.active().unwrap();
    assert!(doc.shows_text() && !doc.readonly);
    assert!(doc.is_dirty());
    assert_eq!(
        doc.text().line_content(0).slice().to_string(),
        "greeting=\\u4F60\\u597D"
    );

    // 撤销：回到打开时的文字。
    app.run_action(crate::keymap::Action::Undo);
    assert!(!app.editor.active().unwrap().is_dirty());
    assert_eq!(
        app.editor
            .active()
            .unwrap()
            .text()
            .line_content(1)
            .slice()
            .to_string(),
        "farewell=\\u518D\\u89C1 # \\u6CE8\\u91CA"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 分栏和预览按 Properties 种类记忆。窗口恢复时不改变其他种类的首选方式。
#[test]
fn properties_remembers_its_own_preview_mode() {
    let (mut app, dir) = properties("properties-remembered");
    let second = dir.join("other.properties");
    let markdown = dir.join("notes.md");
    std::fs::write(&second, "a=\\u4E00\n").unwrap();
    std::fs::write(&markdown, "# 标题\n").unwrap();

    app.choose_preview_mode(PreviewMode::Split);
    let saved = |app: &App| app.state.preview_modes.get("properties").cloned();
    assert_eq!(saved(&app).as_deref(), Some("split"));
    app.editor.preview_defaults[PreviewKind::Properties as usize] = PreviewMode::Source;
    app.apply_preview_state();
    assert_eq!(
        app.editor.preview_defaults[PreviewKind::Properties as usize],
        PreviewMode::Split
    );

    app.open_path(&second);
    paint(&mut app);
    assert_eq!(
        app.editor.active().unwrap().preview_mode(),
        PreviewMode::Split
    );
    app.choose_preview_mode(PreviewMode::Preview);
    assert_eq!(saved(&app).as_deref(), Some("preview"));
    // 其他预览种类不受影响。
    app.open_path(&markdown);
    paint(&mut app);
    assert_eq!(
        app.editor.active().unwrap().preview_mode(),
        PreviewMode::Source
    );

    // 新窗口按状态恢复：Properties 只看预览，Markdown 仍是源代码。
    app.editor.preview_defaults = [PreviewMode::Source; PreviewKind::ALL.len()];
    app.apply_preview_state();
    assert_eq!(
        app.editor.preview_defaults,
        [
            PreviewMode::Source,
            PreviewMode::Source,
            PreviewMode::Source,
            PreviewMode::Source,
            PreviewMode::Preview
        ]
    );

    // 改回代码：状态清空。
    app.open_path(&second);
    paint(&mut app);
    app.choose_preview_mode(PreviewMode::Source);
    assert_eq!(saved(&app), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 源码自动换行时，预览按物理行定位。键盘翻页后，切回源码仍在同一行附近。
#[test]
fn properties_preview_keeps_the_source_line_and_scrolls_with_keys() {
    use crate::keymap::{KeyCode, Mods, Stroke};
    use winit::keyboard::{Key, NamedKey};

    use_vscode_keys();
    let command = if cfg!(target_os = "macos") {
        Mods::META
    } else {
        Mods::CTRL
    };
    let toggle = Stroke {
        key: KeyCode::Char('v'),
        mods: command | Mods::SHIFT,
    };
    let dir = sample_folder("properties-wrapped-preview");
    let path = dir.join("long.properties");
    let mut source = format!("long={}\n", "long word ".repeat(400));
    for index in 0..100 {
        source.push_str(&format!("key{index}=\\u4F60\\u597D\n"));
    }
    std::fs::write(&path, source).unwrap();
    let mut app = app_with(&path, false);
    app.config.word_wrap = Some(true);
    paint(&mut app);

    // 分栏线和左侧滚动条相邻，滑块中心仍然用于滚动源码。
    click_mode(&mut app, PreviewMode::Split);
    let track = app.scroll_track();
    let thumb = scrollbar_thumb(track, 0.0, app.content_height()).unwrap();
    assert_eq!(
        app.hover_at(thumb.x + thumb.w / 2.0, thumb.y + thumb.h / 2.0),
        Hover::Scrollbar(Scrollbar::Editor)
    );
    assert_eq!(
        app.hover_at(app.layout.split.x - 0.5, thumb.y + thumb.h / 2.0),
        Hover::SplitDivider
    );
    click_mode(&mut app, PreviewMode::Source);

    let doc = app.editor.active().unwrap();
    let row = doc.row_of(1);
    assert!(row > 1);
    let selection = doc.selection;
    app.set_active_scroll(row as f64);

    assert!(app.on_shortcut(toggle));
    paint(&mut app);
    assert_eq!(app.preview.properties.top_line, 1.0);
    let pane = app.layout.editor;
    click(&mut app, (pane.x + pane.w / 2.0, pane.y + pane.h / 2.0));
    assert!(app.properties_key(&Key::Named(NamedKey::PageDown), false));
    let shown = app.preview.properties.top_line.floor() as usize;
    assert!(shown > 1);

    let other = dir.join("other.properties");
    std::fs::write(&other, "key=other\n").unwrap();
    app.open_path(&other);
    paint(&mut app);
    app.open_path(&path);
    paint(&mut app);
    assert_eq!(app.preview.properties.top_line.floor() as usize, shown);

    assert!(app.on_shortcut(toggle));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.line_of_row(doc.scroll_line as usize), shown);
    assert_eq!(doc.selection, selection);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 切回代码并滚动后再次打开分栏：右侧预览从左侧当前的阅读行开始，不回到旧位置。
#[test]
fn reopening_split_starts_the_preview_at_the_source_line() {
    let dir = sample_folder("properties-split-reopen");
    let path = dir.join("long.properties");
    std::fs::write(&path, "key=\\u4F60\\u597D\n".repeat(120)).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);

    click_mode(&mut app, PreviewMode::Split);
    click_mode(&mut app, PreviewMode::Source);
    app.set_active_scroll(51.0);
    paint(&mut app);
    click_mode(&mut app, PreviewMode::Split);
    assert_eq!(app.preview.properties.top_line.floor() as usize, 51);
    assert_eq!(app.properties_area(), Some(app.layout.split));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 分栏因窗口太窄被隐藏后切到预览：从源码当前的阅读行开始，而不是回到旧位置。
#[test]
fn switching_a_hidden_split_to_preview_keeps_the_source_line() {
    use winc_render::Renderer;

    /// 把离屏界面的窗口改为 `size`（逻辑像素）并重新布局。
    fn resize(app: &mut App, size: (f32, f32)) {
        let background = app.palette.frame_bg;
        let renderer = Renderer::offscreen(size.0, size.1, SCALE, background, None, None).unwrap();
        app.frame.renderer = Some(renderer);
        app.relayout();
        app.clamp_scroll();
    }

    let dir = sample_folder("properties-split-hidden");
    let path = dir.join("long.properties");
    std::fs::write(&path, "key=\\u4F60\\u597D\n".repeat(120)).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);

    click_mode(&mut app, PreviewMode::Split);
    assert_eq!(app.properties_area(), Some(app.layout.split));
    // 窗口缩到分栏放不下：右侧预览隐藏，模式仍是分栏。
    resize(&mut app, (640.0, 600.0));
    paint(&mut app);
    assert_eq!(
        app.editor.active().unwrap().preview_mode(),
        PreviewMode::Split
    );
    assert!(app.properties_area().is_none());
    assert_eq!(app.layout.split.w, 0.0);

    app.set_active_scroll(51.0);
    paint(&mut app);
    app.set_preview_mode(PreviewMode::Preview);
    paint(&mut app);
    assert_eq!(app.preview.properties.top_line.floor() as usize, 51);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 同一行在源码中更宽，预览的横向滚动范围以解码后的文字为准。
#[test]
fn properties_preview_scrolls_only_as_far_as_the_decoded_text() {
    let dir = sample_folder("properties-horizontal-preview");
    let path = dir.join("long.properties");
    let reference = dir.join("decoded.properties");
    // 原文超出编辑器的默认显示上限，解码后的译文仍能整行显示。
    std::fs::write(&path, format!("key={}\n", "\\u4F60".repeat(2_000))).unwrap();
    std::fs::write(&reference, format!("key={}\n", "你".repeat(2_000))).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    let source_max = app.text_max_scroll_x();
    assert!(source_max > 0.0);
    app.editor.active_mut().unwrap().scroll_x = source_max;

    click_mode(&mut app, PreviewMode::Preview);
    let max = app.preview.properties.max_scroll_x;
    assert!(max > 0.0 && max < source_max);
    let mut decoded = app_with(&reference, false);
    paint(&mut decoded);
    decoded.set_preview_mode(PreviewMode::Preview);
    paint(&mut decoded);
    assert_eq!(max, decoded.preview.properties.max_scroll_x);
    let pane = app.layout.editor;
    let origin = (pane.x + pane.w / 2.0, pane.y + pane.h / 2.0);
    let target = app.scroll_target_at(origin.0, origin.1);
    app.scroll_target_by(target, origin, false, (-source_max * 2.0, 0.0));
    assert_eq!(app.preview.properties.scroll_x, max);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 同一个文档在多个窗格里共用预览模式，每个窗格沿用自己的阅读位置。
#[test]
fn properties_preview_aligns_each_pane_after_switching_modes() {
    use crate::keymap::Action;
    use winit::keyboard::{Key, NamedKey};

    let dir = sample_folder("properties-pane-preview");
    let path = dir.join("long.properties");
    std::fs::write(&path, "key=\\u4F60\n".repeat(120)).unwrap();
    let mut app = app_with(&path, false);
    let left = app.editor.panes.focused;
    assert!(app.run_action(Action::SplitRight));
    let right = app.editor.panes.focused;
    app.set_preview_mode(PreviewMode::Preview);
    paint(&mut app);
    assert!(app.properties_key(&Key::Named(NamedKey::PageDown), false));
    assert!(app.preview.properties.top_line > 15.0);

    app.focus_pane(left);
    app.set_preview_mode(PreviewMode::Source);
    app.with_pane(right, |app| app.set_active_scroll(15.0));
    app.set_preview_mode(PreviewMode::Preview);
    paint(&mut app);
    assert_eq!(app.preview.properties.top_line, 0.0);
    assert_eq!(
        app.with_pane(right, |app| app.preview.properties.top_line),
        15.0
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 拖动分栏分隔线和预览的两条滚动条都能改变各自的滚动位置与宽度。
#[test]
fn properties_preview_drags_its_scrollbars_and_split_divider() {
    use winc_editor::PreviewMode;
    use winit::event::MouseButton;

    let dir = sample_folder("properties-drag");
    let path = dir.join("long.properties");
    let mut source = format!("long={}\n", "\\u4F60".repeat(2_000));
    for index in 0..120 {
        source.push_str(&format!("key{index}=\\u4F60\\u597D\n"));
    }
    std::fs::write(&path, source).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);

    // 分栏：拖动分隔线，预览一侧随之变宽。
    click_mode(&mut app, PreviewMode::Split);
    let before = app.layout.split.w;
    let divider = app.layout.split_divider;
    let (x, y) = (divider.x + divider.w / 2.0, divider.y + divider.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.pointer.hover, Hover::SplitDivider);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x - 60.0, y);
    app.on_mouse_up(MouseButton::Left);
    paint(&mut app);
    assert!(app.layout.split.w > before);
    assert_eq!(app.properties_area(), Some(app.layout.split));

    // 只看预览：先拖动水平滚动条，此时最长的解码行在顶部。
    click_mode(&mut app, PreviewMode::Preview);
    let (track, _, content) = app.properties_hscrollbar().unwrap();
    assert!(content > track.w);
    assert!(hscrollbar_thumb(track, 0.0, content).is_some());
    let y = track.bottom() - 4.0;
    app.on_mouse_move(track.x + 8.0, y);
    assert!(matches!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::PropertiesHorizontal)
    ));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(track.right(), y);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.preview.properties.scroll_x > 0.0);

    // 拖动竖直滚动条到底部，顶部行随之移动。
    let (track, _, content) = app.properties_vscrollbar().unwrap();
    let thumb = scrollbar_thumb(track, 0.0, content).unwrap();
    let x = track.right() - 4.0;
    app.on_mouse_move(x, thumb.y + thumb.h / 2.0);
    assert!(matches!(
        app.pointer.hover,
        Hover::Scrollbar(Scrollbar::Properties)
    ));
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, track.bottom());
    app.on_mouse_up(MouseButton::Left);
    assert!(app.preview.properties.top_line > 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 预览期间关闭有未保存修改的标签并选「保存」：写回源代码的转义写法，而不是预览中显示的
/// 文字，然后照常关闭。
#[test]
fn a_previewing_properties_tab_saves_its_source_when_closing() {
    let (mut app, dir) = properties("properties-save-in-preview");
    let path = dir.join("messages_zh.properties");
    app.run_action(crate::keymap::Action::SelectAll);
    app.run(Command::Insert("greeting=\\u4F60\\u597D\n".into()));
    app.set_preview_mode(PreviewMode::Preview);
    paint(&mut app);
    assert!(app.editor.active().unwrap().readonly);

    app.close_tab(0);
    assert!(app.closing.prompt.is_some());
    app.answer_save_prompt(SaveChoice::Save);
    assert!(app.editor.documents.is_empty());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "greeting=\\u4F60\\u597D\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
