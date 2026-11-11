//! Vim 模式的界面测试：开关、模式与光标形状、状态栏、各模式下是否接受文字输入、撤销分组，
//! 以及与鼠标、应用命令、标签切换的配合。

use winc_core::Selection;
use winc_editor::Command;
use winc_vim::{Code, Mode, Named, notation};

use super::whitespace::changed;
use super::*;
use crate::app::vim::CaretShape;
use crate::keymap::Action;

/// 创建打开了内容为 `text` 的文件、并已开启 Vim 模式的界面；返回界面与样本目录。
fn vim_app(name: &str, text: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("vim.txt");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    // 直接改配置：经设置开关会开始一段动画，之后各帧的像素随时间变化。
    app.config.vim_enabled = Some(true);
    app.vim_apply_config();
    paint(&mut app);
    (app, dir)
}

/// 依次输入按键记法 `keys`；Vim 层不处理的按键按编辑区原有的方式输入文字。
fn keys(app: &mut App, keys: &str) {
    for key in notation::parse(keys) {
        if app.vim_press(key) {
            continue;
        }
        match key.code() {
            Code::Char(c) if key.as_char().is_some() => app.run(Command::Insert(c.to_string())),
            Code::Named(Named::Enter) => app.run(Command::Newline),
            Code::Named(Named::Backspace) => app.run(Command::Backspace { word: false }),
            _ => {}
        }
    }
}

/// 返回当前文档的文本。
fn text(app: &App) -> String {
    let doc = app.editor.active().unwrap();
    doc.text_range(0, doc.text().end_pos()).unwrap()
}

/// 返回当前文档主选区的端点。
fn head(app: &App) -> usize {
    app.editor.active().unwrap().selection.head
}

/// 绘制一帧并返回全部像素。
fn pixels(app: &mut App) -> Vec<u32> {
    paint(app);
    let renderer = app.renderer.as_ref().unwrap();
    renderer.image().unwrap().pixels.clone()
}

/// 返回光标占用的像素数：光标亮、灭、再亮三帧中，随光标亮灭而变化的像素。
///
/// 后台任务（文件树、Git 状态）的结果可能在两帧之间到达；只在一次切换中变化的像素不计入。
fn caret_pixels(app: &mut App) -> usize {
    app.blink.visible = true;
    let lit = pixels(app);
    app.blink.visible = false;
    let dark = pixels(app);
    app.blink.visible = true;
    let again = pixels(app);
    (lit.iter().zip(&dark).zip(&again))
        .filter(|((lit, dark), again)| lit != dark && lit == again)
        .count()
}

#[test]
fn vim_mode_is_off_by_default_and_keeps_no_state() {
    let dir = sample_folder("vim-off");
    let mut app = app_with(&dir.join("a.txt"), false);
    assert!(app.vim.is_none());
    assert_eq!(
        (app.vim_mode(), app.vim_caret(), app.vim_status()),
        (None, None, None)
    );
    // 关闭时按键不经 Vim 层，编辑区仍然接受文字。
    assert!(!app.vim_press(winc_vim::Key::char('x')));
    assert!(app.takes_text());

    // 设置页的开关与命令都能开启和关闭，并写入配置。
    app.settings_toggle(Setting::VimMode);
    assert!(app.vim.is_some());
    assert_eq!(app.config.vim_enabled, Some(true));
    assert!(app.run_action(Action::ToggleVimMode));
    assert!(app.vim.is_none());
    assert_eq!(app.config.vim_enabled, Some(false));
    assert!(app.takes_text());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn modes_decide_text_input_the_status_bar_and_the_caret() {
    let (mut app, dir) = vim_app("vim-modes", "one two\nthree\n");

    // 普通模式：不接受文字输入（输入法停用），块状光标，状态栏不显示模式。
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    assert!(!app.takes_text());
    assert_eq!(app.vim_caret(), Some((0, CaretShape::Block)));
    assert_eq!(app.vim_status(), None);

    // 等待动作：下划线光标，状态栏显示已输入的按键。
    keys(&mut app, "2d");
    assert_eq!(app.vim_caret(), Some((0, CaretShape::Underline)));
    assert_eq!(app.vim_status().as_deref(), Some("2d"));
    assert!(!app.takes_text());
    keys(&mut app, "<Esc>");
    assert_eq!(app.vim_status(), None);

    // 插入模式：接受文字输入，使用竖线光标。
    keys(&mut app, "i");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    assert!(app.takes_text());
    assert_eq!(app.vim_caret(), None);
    assert_eq!(app.vim_status().as_deref(), Some("-- 插入 --"));
    keys(&mut app, "<Esc>");
    assert!(!app.takes_text());

    // 可视模式：光标在选区的光标一端所含的字符上。
    keys(&mut app, "vll");
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    assert!(!app.takes_text());
    assert_eq!(app.editor.active().unwrap().selection, Selection::new(0, 3));
    assert_eq!(app.vim_caret(), Some((2, CaretShape::Block)));
    assert_eq!(app.vim_status().as_deref(), Some("-- 可视 --"));
    keys(&mut app, "V\"a");
    assert_eq!(app.vim_status().as_deref(), Some("-- 可视 行 --  \"a"));
    keys(&mut app, "<Esc><Esc>");
    assert_eq!(
        (app.vim_mode(), app.vim_status()),
        (Some(Mode::Normal), None)
    );

    // 焦点离开编辑区：空心的块状光标，其他输入框仍然接受文字。
    app.focus = Focus::Tree;
    assert_eq!(
        app.vim_caret().map(|caret| caret.1),
        Some(CaretShape::Hollow)
    );
    assert_eq!(app.vim_mode(), None);
    app.focus = Focus::Search;
    assert!(app.takes_text());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_caret_is_drawn_as_a_block_a_bar_or_an_underline() {
    let (mut app, dir) = vim_app("vim-caret", "one two\nthree\n");
    // 窗口获得焦点后先绘制一帧，之后两帧之间只有光标不同。
    app.window.focused = true;
    paint(&mut app);
    let block = caret_pixels(&mut app);
    keys(&mut app, "d");
    let underline = caret_pixels(&mut app);
    keys(&mut app, "<Esc>i");
    let bar = caret_pixels(&mut app);
    keys(&mut app, "<Esc>");

    // 块覆盖一个字符格，竖线宽 2 逻辑像素，横线高 2 逻辑像素、宽一个字符格。
    let scale = (SCALE * SCALE) as usize;
    let cell = app.cell.round() as usize;
    let line = app.fonts.line_height as usize;
    assert_eq!(bar, 2 * (line - 2) * scale);
    assert_eq!(underline, cell * 2 * scale);
    assert!(block > bar * 2 && block <= cell * line * scale, "{block}");

    // 空心块不闪烁，只画四条边：宽一个字符格、高一行的边框。
    app.focus = Focus::Tree;
    assert_eq!(caret_pixels(&mut app), 0);
    let hollow = pixels(&mut app);
    app.config.vim_enabled = Some(false);
    app.vim_apply_config();
    let none = pixels(&mut app);
    let edges = changed(&hollow, &none);
    assert!(edges > 0 && edges < block, "{edges}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_change_and_its_typed_text_undo_together_and_repeat() {
    let (mut app, dir) = vim_app("vim-undo", "one two three\nfour\n");
    keys(&mut app, "cwab cd<Esc>");
    assert_eq!(text(&app), "ab cd two three\nfour\n");
    assert_eq!((app.vim_mode(), head(&app)), (Some(Mode::Normal), 4));
    // 点重复重放命令与键入的文字。
    keys(&mut app, "ww.");
    assert_eq!(text(&app), "ab cd two ab cd\nfour\n");
    // 每次修改连同键入的文字是一个撤销单元。
    keys(&mut app, "u");
    assert_eq!(text(&app), "ab cd two three\nfour\n");
    keys(&mut app, "u");
    assert_eq!(
        (text(&app).as_str(), head(&app)),
        ("one two three\nfour\n", 0)
    );
    keys(&mut app, "<C-r>");
    assert_eq!(text(&app), "ab cd two three\nfour\n");

    // 新开的行沿用所在行的缩进；带次数时重复输入，整体是一个撤销单元。
    keys(&mut app, "uI    <Esc>");
    keys(&mut app, "2ox<Esc>");
    assert_eq!(text(&app), "    one two three\n    x\n    x\nfour\n");
    keys(&mut app, "jOy<Esc>");
    assert_eq!(text(&app), "    one two three\n    x\n    x\ny\nfour\n");
    keys(&mut app, "uu");
    assert_eq!(text(&app), "    one two three\nfour\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn alt_shortcuts_of_the_application_stay_with_it() {
    use crate::keymap::{KeyCode, Mods, Stroke};
    let (app, dir) = vim_app("vim-alt", "abc\n");
    // `Alt`+上（上移一行）是应用的快捷键；`Alt`+`J` 没有绑定，仍交给 Vim 层。
    let (up, j) = (KeyCode::Up, KeyCode::Char('j'));
    assert!(app.binds(Stroke {
        key: up,
        mods: Mods::ALT
    }));
    assert!(!app.binds(Stroke {
        key: j,
        mods: Mods::ALT
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_without_a_meaning_never_become_text() {
    let (mut app, dir) = vim_app("vim-keys", "abc\n");
    // 普通模式下没有定义的可见字符被吞掉，功能键与 `Esc` 交给应用。
    assert!(app.vim_press(winc_vim::Key::char('\\')));
    assert!(app.vim_press(notation::parse("<Tab>")[0]));
    assert!(!app.vim_press(notation::parse("<Esc>")[0]));
    assert!(!app.vim_press(notation::parse("<PageDown>")[0]));
    assert_eq!(text(&app), "abc\n");

    // 尚未实现的命令只在状态栏提示。
    keys(&mut app, ":");
    assert_eq!(
        app.status.message.as_deref(),
        Some("Vim 模式尚不支持此命令")
    );
    assert_eq!(
        (text(&app).as_str(), app.vim_mode()),
        ("abc\n", Some(Mode::Normal))
    );

    // 只读文档：修改类命令提示原因，不改变模式；移动与复制仍然可用。
    app.editor.active_mut().unwrap().readonly = true;
    app.status.message = None;
    keys(&mut app, "x");
    assert!(app.status.message.is_some());
    keys(&mut app, "i");
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    keys(&mut app, "l\"+yl");
    assert_eq!((head(&app), app.copied.as_deref()), (1, Some("b")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn selections_and_commands_from_outside_keep_the_mode_in_step() {
    let (mut app, dir) = vim_app("vim-outside", "one two\nthree\n");

    // 鼠标拖出选区：普通模式进入可视模式，之后的命令作用于选区。
    app.editor.active_mut().unwrap().selection = Selection::new(4, 7);
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Visual));
    assert_eq!(app.vim_caret(), Some((6, CaretShape::Block)));
    // 应用的复制等同于复制到系统剪贴板，随后回到普通模式。
    assert!(app.run_action(Action::Copy));
    assert_eq!(app.copied.as_deref(), Some("two"));
    assert_eq!((app.vim_mode(), head(&app)), (Some(Mode::Normal), 4));

    // 点击行尾之后：光标退到最后一个字符。
    app.editor.active_mut().unwrap().selection = Selection::point(7);
    paint(&mut app);
    assert_eq!((app.vim_mode(), head(&app)), (Some(Mode::Normal), 6));

    // 应用的剪切、粘贴、撤销、重做由 Vim 层执行。
    assert!(app.run_action(Action::Cut));
    assert_eq!(
        (text(&app).as_str(), app.copied.as_deref()),
        ("three\n", Some("one two\n"))
    );
    assert!(app.run_action(Action::Paste));
    assert_eq!((text(&app).as_str(), head(&app)), ("one two\nthree\n", 8));
    assert!(app.run_action(Action::Undo));
    assert_eq!(text(&app), "three\n");
    assert!(app.run_action(Action::Redo));
    assert_eq!(text(&app), "one two\nthree\n");

    // 插入模式下选区不改变模式，应用的命令照原样执行；光标被移动之后另起一个撤销单元。
    keys(&mut app, "ggiab");
    app.editor.active_mut().unwrap().selection = Selection::new(3, 6);
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    keys(&mut app, "X<Esc>");
    assert_eq!(text(&app), "aboXtwo\nthree\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "abone two\nthree\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "one two\nthree\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_tabs_returns_to_normal_mode_and_ends_the_insert() {
    let (mut app, dir) = vim_app("vim-tabs", "first\n");
    let first = dir.join("vim.txt");
    let second = dir.join("a.txt");
    keys(&mut app, "ix");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));

    app.open_path(&second);
    paint(&mut app);
    assert_eq!(app.vim_mode(), Some(Mode::Normal));
    assert!(!app.takes_text());

    // 原文档的插入已经结束：回去之后的两次修改各是一个撤销单元。
    app.open_path(&first);
    paint(&mut app);
    assert_eq!(text(&app), "xfirst\n");
    keys(&mut app, "$xx");
    assert_eq!(text(&app), "xfir\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "xfirs\n");
    keys(&mut app, "uu");
    assert_eq!(text(&app), "first\n");

    // 关闭 Vim 模式时同样结束进行中的插入。
    keys(&mut app, "iy");
    app.settings_toggle(Setting::VimMode);
    assert!(app.vim.is_none() && app.takes_text());
    app.run(Command::Insert("z".into()));
    app.run(Command::Undo);
    assert_eq!(text(&app), "yfirst\n");

    let _ = std::fs::remove_dir_all(&dir);
}
