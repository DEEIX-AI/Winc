//! Vim 模式的界面测试：开关、模式与光标形状、状态栏、各模式下是否接受文字输入、撤销分组，
//! 以及与鼠标、应用命令、标签切换的配合。

use winc_core::Selection;
use winc_editor::Command;
use winc_vim::{Code, Mode, Named, notation};

use super::whitespace::changed;
use super::*;
use crate::app::vim::CaretShape;
use crate::app::{Focus, PromptKind};
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
    assert!(!app.vim_press(notation::parse("<F5>")[0]));
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

/// 返回光标所在的行号。
fn line(app: &App) -> usize {
    let doc = app.editor.active().unwrap();
    doc.text().line_of(doc.selection.head)
}

#[test]
fn searching_uses_the_prompt_previews_the_match_and_keeps_a_history() {
    let (mut app, dir) = vim_app("vim-search", "one two\nthree two\nfour\n");
    keys(&mut app, "/");
    assert_eq!(app.focus, Focus::Prompt);
    assert_eq!(app.prompt_kind, PromptKind::VimSearch { forward: true });
    assert_eq!(app.vim_mode(), None);

    // 输入过程中光标即时移到第一处匹配，匹配被高亮；回车之后停在该处。
    app.edit_field(|field| field.insert("two"));
    paint(&mut app);
    assert_eq!(head(&app), 4);
    assert!(app.vim_highlights());
    app.submit_prompt();
    assert_eq!((app.focus, head(&app)), (Focus::Editor, 4));
    keys(&mut app, "n");
    assert_eq!(head(&app), 14);
    keys(&mut app, "N");
    assert_eq!(head(&app), 4);

    // 再次打开时可以翻出上一次的模式；放弃输入时光标回到原处，高亮取消。
    keys(&mut app, "?");
    assert_eq!(app.prompt_kind, PromptKind::VimSearch { forward: false });
    app.edit_field(|field| field.recall(true));
    assert_eq!(app.goto_input.as_ref().unwrap().text(), "two");
    paint(&mut app);
    assert_eq!(head(&app), 14);
    app.close_prompt();
    app.focus = Focus::Editor;
    app.vim_prompt_cancel();
    assert_eq!(head(&app), 4);
    assert!(!app.vim_highlights());

    // 查找可以作为操作符的范围；找不到与无法转换的模式只提示。
    keys(&mut app, "d/");
    app.edit_field(|field| field.insert("four"));
    app.submit_prompt();
    assert_eq!(text(&app), "one \nfour\n");
    for (pattern, message) in [
        ("zzz", "找不到模式"),
        ("a\\%Vb", "模式的第 2 个字符处有无法识别的写法"),
    ] {
        keys(&mut app, "/");
        app.edit_field(|field| field.insert(pattern));
        app.submit_prompt();
        assert_eq!(app.status.message.as_deref(), Some(message), "{pattern}");
    }
    assert_eq!(text(&app), "one \nfour\n");

    // 查找条打开之后以查找条的内容为准。
    keys(&mut app, "0*");
    assert!(app.vim_highlights());
    app.open_search(false);
    assert!(!app.vim_highlights());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn marks_follow_edits_and_jumps_use_the_position_history() {
    let body: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    let (mut app, dir) = vim_app("vim-marks", &format!("first\n\nsecond\n{body}"));
    app.track_navigation();

    // 标记随其上方插入的行移动。
    keys(&mut app, "jjma");
    let marked = head(&app);
    keys(&mut app, "ggOnew<Esc>");
    keys(&mut app, "`a");
    assert_eq!(head(&app), marked + 4);
    keys(&mut app, "`z");
    assert_eq!(app.status.message.as_deref(), Some("标记 z 没有设置"));

    // 跳转之前的位置记入位置历史：`Ctrl-O` 后退，`Tab` 前进；不远的跳转同样记入。每批事件
    // 处理完之后记录一次位置。
    let step = |app: &mut App, typed: &str| {
        keys(app, typed);
        app.track_navigation();
    };
    step(&mut app, "gg");
    step(&mut app, "G");
    let last = line(&app);
    step(&mut app, "<C-o>");
    assert_eq!(line(&app), 0);
    step(&mut app, "<Tab>");
    assert_eq!(line(&app), last);
    step(&mut app, "gg");
    step(&mut app, "}");
    assert_eq!(line(&app), 2);
    step(&mut app, "<C-o>");
    assert_eq!(line(&app), 0);
    // 没有移动光标的跳转命令不留下记录。
    step(&mut app, "gg");
    step(&mut app, "j");
    step(&mut app, "<C-o>");
    assert_eq!(line(&app), 43);

    // 修改列表：回到最近修改的位置。
    keys(&mut app, "Gx");
    let changed = head(&app);
    keys(&mut app, "ggg;");
    assert_eq!(head(&app), changed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scrolling_commands_move_the_window_and_keep_the_caret_inside() {
    let body: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    let (mut app, dir) = vim_app("vim-scroll", &body);
    let rows = app.visible_lines() as f64;
    let scroll = |app: &App| app.editor.active().unwrap().scroll_line;

    // 所在行移到窗口的顶部、中间、底部；顶部留出一行。
    keys(&mut app, "100G");
    keys(&mut app, "zt");
    assert_eq!(scroll(&app), 98.0);
    keys(&mut app, "zz");
    assert_eq!(scroll(&app), 99.0 - (rows / 2.0).floor());
    keys(&mut app, "zb");
    assert_eq!(scroll(&app), 99.0 - rows + 2.0);
    assert_eq!(line(&app), 99);

    // 按行滚动：光标仍在窗口之内时不动，滚出窗口时跟随。
    keys(&mut app, "zz");
    let before = scroll(&app);
    keys(&mut app, "<C-e>");
    assert_eq!((scroll(&app), line(&app)), (before + 1.0, 99));
    keys(&mut app, "zt");
    keys(&mut app, "3<C-e>");
    assert!(line(&app) > 99, "{}", line(&app));

    // 半屏与整屏：光标随窗口移动。
    keys(&mut app, "100Gzz");
    let half = (app.visible_lines() / 2).max(1);
    keys(&mut app, "<C-d>");
    assert_eq!(line(&app), 99 + half);
    keys(&mut app, "<C-u>");
    assert_eq!(line(&app), 99);
    let before = scroll(&app);
    keys(&mut app, "<C-f>");
    assert_eq!(scroll(&app), before + rows - 2.0);
    keys(&mut app, "<PageUp>");
    assert_eq!(scroll(&app), before);

    // 光标与窗口上下边缘之间保留配置的行数。
    app.config.vim_scroll_off = Some(5);
    keys(&mut app, "150G");
    for _ in 0..40 {
        keys(&mut app, "k");
    }
    let row = app.editor.active().unwrap().row_at(head(&app)) as f64;
    assert!(row - scroll(&app) >= 5.0, "{row} {}", scroll(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fold_commands_use_the_blocks_of_the_document() {
    let dir = sample_folder("vim-folds");
    let path = dir.join("fold.rs");
    std::fs::write(
        &path,
        "fn a() {\n    x;\n    if y {\n        z;\n    }\n}\nfn b() {\n    w;\n}\n",
    )
    .unwrap();
    let mut app = app_with(&path, false);
    app.config.vim_enabled = Some(true);
    app.vim_apply_config();
    paint(&mut app);
    let folded = |app: &App| app.editor.active().unwrap().folded();

    // 收起光标所在的块，光标移到块的首行；展开后恢复。
    keys(&mut app, "jzc");
    assert_eq!(folded(&app), [(0, 4)]);
    assert_eq!(line(&app), 0);
    keys(&mut app, "za");
    assert!(folded(&app).is_empty());
    keys(&mut app, "zM");
    assert_eq!(folded(&app).len(), 3);
    keys(&mut app, "zR");
    assert!(folded(&app).is_empty());

    // 在可折叠的块之间移动；收起的块在上下移动中算一行。
    keys(&mut app, "ggzj");
    assert_eq!(line(&app), 2);
    keys(&mut app, "zj");
    assert_eq!(line(&app), 6);
    keys(&mut app, "zk");
    assert_eq!(line(&app), 4);
    keys(&mut app, "ggzcj");
    assert_eq!(line(&app), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replace_mode_overwrites_and_insert_mode_handles_control_keys() {
    let (mut app, dir) = vim_app("vim-replace", "abcdef\nxyz\n");
    keys(&mut app, "R");
    assert_eq!(app.vim_mode(), Some(Mode::Insert));
    assert_eq!(app.vim_status().as_deref(), Some("-- 替换 --"));
    assert_eq!(app.vim_caret().map(|c| c.1), Some(CaretShape::Underline));
    assert!(app.takes_text());
    keys(&mut app, "XY");
    assert_eq!(text(&app), "XYcdef\nxyz\n");
    // 退格还原被覆盖的字符。
    keys(&mut app, "<BS>");
    assert_eq!(text(&app), "Xbcdef\nxyz\n");
    keys(&mut app, "<Esc>j.");
    assert_eq!(text(&app), "Xbcdef\nXyz\n");

    // 插入模式的 `Ctrl` 键：删除前一个词、插入寄存器、执行一条普通模式的命令。
    keys(&mut app, "ggyiwA<Space>one<Space>two<C-w><C-r>0<Esc>");
    assert_eq!(text(&app), "Xbcdef one Xbcdef\nXyz\n");
    keys(&mut app, "A!<C-o>");
    assert_eq!(app.vim_status().as_deref(), Some("-- (插入) --"));
    keys(&mut app, "0?<Esc>");
    assert_eq!(text(&app), "?Xbcdef one Xbcdef!\nXyz\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn block_selections_and_multiple_cursors_edit_every_line() {
    let (mut app, dir) = vim_app("vim-block", "abcd\nefgh\nijkl\n");
    let cursors = |app: &App| app.editor.active().unwrap().selections().0;

    // 按块的选区每行一个；插入对各行同时生效，离开后只留一个光标。
    keys(&mut app, "l<C-v>jl");
    assert_eq!(app.vim_status().as_deref(), Some("-- 可视 块 --"));
    assert_eq!(cursors(&app), [Selection::new(1, 3), Selection::new(6, 8)]);
    keys(&mut app, "IX<Esc>");
    assert_eq!(text(&app), "aXbcd\neXfgh\nijkl\n");
    assert_eq!(cursors(&app), [Selection::point(1)]);
    keys(&mut app, "l<C-v>jld");
    assert_eq!(text(&app), "aXd\neXh\nijkl\n");
    keys(&mut app, "u");
    assert_eq!(text(&app), "aXbcd\neXfgh\nijkl\n");

    // 多个光标：命令在每个光标上各执行一次，粘贴时逐个对应。
    let doc = app.editor.active_mut().unwrap();
    doc.set_selections(vec![Selection::point(0), Selection::point(6)], 0);
    keys(&mut app, "x");
    assert_eq!(text(&app), "Xbcd\nXfgh\nijkl\n");
    assert_eq!(cursors(&app).len(), 2);
    keys(&mut app, "$p");
    assert_eq!(text(&app), "Xbcda\nXfghe\nijkl\n");
    keys(&mut app, "iZ<Esc>");
    assert_eq!(text(&app), "XbcdZa\nXfghZe\nijkl\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_clipboard_setting_decides_what_reaches_the_system_clipboard() {
    use crate::layout::{Setting, SettingControl};
    let (mut app, dir) = vim_app("vim-clipboard", "one\ntwo\n");
    assert_eq!(Setting::VimClipboard.control(), SettingControl::Choice);
    keys(&mut app, "yy");
    assert_eq!(app.copied, None);
    app.set_vim_clipboard(winc_config::VimClipboard::Yank);
    keys(&mut app, "yy");
    assert_eq!(app.copied.as_deref(), Some("one\n"));
    keys(&mut app, "jdd");
    assert_eq!(app.copied.as_deref(), Some("one\n"));
    app.set_vim_clipboard(winc_config::VimClipboard::Always);
    keys(&mut app, "x");
    assert_eq!(app.copied.as_deref(), Some("o"));
    let _ = std::fs::remove_dir_all(&dir);
}
