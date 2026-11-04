//! 编辑区行为的测试：剪贴板、右键菜单、自动保存、`.editorconfig`、状态栏的缩进与编码、
//! 滚动条标记、书签、CSV 列对齐、自动换行。

use super::*;
use crate::keymap::Action;

#[test]
fn copying_without_a_selection_copies_and_pastes_whole_lines() {
    let dir = search_folder("clipboard");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    let line = |app: &App, n: usize| {
        let text = app.editor.active().unwrap().text();
        text.line_content(n).slice().to_string()
    };

    // 光标位于第二行且没有选区：复制整行，包含行尾换行符。
    app.select(15, 15);
    app.copy(false);
    assert_eq!(
        app.copied.as_deref(),
        Some("    let config = load_config();\n")
    );

    // 粘贴整行内容时，插入到光标所在行的上方，而不是光标处。
    app.select(0, 0);
    app.paste();
    assert_eq!(line(&app, 0), "    let config = load_config();");
    assert_eq!(line(&app, 1), "fn main() {");

    // 有选区时复制选区内容，粘贴时按普通文本在光标处插入。
    app.select(4, 7);
    app.copy(false);
    assert_eq!(app.copied.as_deref(), Some("let"));
    app.select(0, 0);
    app.paste();
    assert!(line(&app, 0).starts_with("let    let"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn right_clicking_the_text_offers_format_cut_copy_paste() {
    use MenuCommand::{
        AddBookmark, Copy, CopyReference, Cut, FindReferences, Fix, Format, FormatSelection,
        GoToDefinition, ListBookmarks, Paste, RemoveBookmark,
    };

    let dir = search_folder("editor-menu");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    paint(&mut app);
    // 在第 `line` 行第 `col` 列（均从 0 起）处按右键。
    let right_click_text = |app: &mut App, line: usize, col: usize| {
        let t = app.layout.text;
        let x = t.x + TEXT_PAD + col as f32 * app.cell;
        right_click(app, (x, t.y + (line as f32 + 0.5) * app.fonts.line_height));
    };
    let selection = |app: &App| {
        let s = app.editor.active().unwrap().selection;
        (s.from(), s.to())
    };
    let line = |app: &App, n: usize| {
        let text = app.editor.active().unwrap().text();
        text.line_content(n).slice().to_string()
    };

    // 在选区内右键：保留选区，菜单命令作用于该选区。
    app.select(20, 26);
    right_click_text(&mut app, 1, 10);
    let menu = app.context_menu.as_ref().unwrap();
    assert!(matches!(menu.target, MenuTarget::Editor(_)));
    assert_eq!(
        menu_commands(&app),
        [
            (GoToDefinition, true),
            (FindReferences, true),
            (AddBookmark, true),
            (RemoveBookmark, false),
            (ListBookmarks, true),
            (Format, true),
            (FormatSelection, true),
            (Fix, true),
            (Cut, true),
            (Copy, true),
            (Paste, true),
            (CopyReference, true)
        ]
    );
    assert_eq!(selection(&app), (20, 26));
    app.run_menu_command(Copy);
    assert_eq!(app.copied.as_deref(), Some("config"));

    // 这一行加了书签以后，菜单里换成「删除书签」可用。
    right_click_text(&mut app, 1, 10);
    app.run_menu_command(AddBookmark);
    right_click_text(&mut app, 1, 10);
    let enabled = |app: &App, command| {
        menu_commands(app)
            .into_iter()
            .find(|(c, _)| *c == command)
            .map(|(_, on)| on)
    };
    assert_eq!(enabled(&app, AddBookmark), Some(false));
    assert_eq!(enabled(&app, RemoveBookmark), Some(true));
    app.run_menu_command(RemoveBookmark);
    right_click_text(&mut app, 1, 10);
    assert_eq!(enabled(&app, AddBookmark), Some(true));

    // 在选区外右键：光标移到点击处。没有选区时，剪切的是整行。
    right_click_text(&mut app, 0, 3);
    assert_eq!(selection(&app), (3, 3));
    app.run_menu_command(Cut);
    assert_eq!(app.copied.as_deref(), Some("fn main() {\n"));
    assert_eq!(line(&app, 0), "    let config = load_config();");

    // 粘贴整行：插入到光标所在行的上方。
    right_click_text(&mut app, 0, 0);
    app.run_menu_command(Paste);
    assert_eq!(line(&app, 0), "fn main() {");
    assert_eq!(line(&app, 1), "    let config = load_config();");

    // 查找条覆盖的区域不弹出编辑区菜单，右键事件由查找条自己处理。
    app.open_search(false);
    paint(&mut app);
    let bar = app.layout.search;
    right_click(&mut app, (bar.x + bar.w / 2.0, bar.y + bar.h / 2.0));
    assert!(app.context_menu.is_none());

    // 只读文档：格式化、修复、剪切、粘贴不可用，转到定义、查找引用和复制仍可用。
    app.editor.active_mut().unwrap().readonly = true;
    right_click_text(&mut app, 0, 0);
    assert_eq!(
        menu_commands(&app),
        [
            (GoToDefinition, true),
            (FindReferences, true),
            (AddBookmark, true),
            (RemoveBookmark, false),
            (ListBookmarks, true),
            (Format, false),
            (Fix, false),
            (Cut, false),
            (Copy, true),
            (Paste, false),
            (CopyReference, true),
        ]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_save_writes_on_switching_tabs_and_leaving_the_window() {
    let dir = winc_config::files::canonicalize(sample_folder("auto-save")).unwrap();
    let mut app = app_with(&dir, false);
    let (a, b) = (dir.join("a.txt"), dir.join("short.rs"));
    let read = |path: &Path| std::fs::read_to_string(path).unwrap();
    let original = read(&a);

    // 未开启自动保存：切换标签不保存。
    app.open_path(&a);
    app.run(Command::Insert("x".into()));
    app.open_path(&b);
    assert_eq!(read(&a), original);
    let a_index = app
        .editor
        .documents
        .iter()
        .position(|d| d.path.as_deref() == Some(&*a));

    // 开启自动保存：切换离开时保存，且不在状态栏提示。
    app.config.auto_save = Some(true);
    app.status_message = None;
    app.activate_tab(a_index.unwrap());
    app.run(Command::Insert("y".into()));
    app.open_path(&b);
    assert!(read(&a).starts_with("xy"), "{}", read(&a));
    assert!(app.status_message.is_none());
    assert!(
        app.editor
            .documents
            .iter()
            .all(|d| !d.is_dirty() || d.path.as_deref() == Some(&*b))
    );

    // 窗口失去焦点：保存所有有修改的文档。
    app.run(Command::Insert("z".into()));
    let ids: Vec<DocumentId> = app.editor.documents.iter().map(|d| d.id).collect();
    app.auto_save(&ids);
    assert!(read(&b).starts_with('z'));

    // 磁盘文件已被其他程序修改（修改时间设为当前时间之后 5 秒，保证晚于文档记录的时间）：
    // 自动保存不覆盖，留待用户决定。
    app.open_path(&b);
    app.run(Command::Insert("w".into()));
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::write(&b, "theirs\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&b)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let ids: Vec<DocumentId> = app.editor.documents.iter().map(|d| d.id).collect();
    app.auto_save(&ids);
    assert_eq!(read(&b), "theirs\n");
    assert!(app.editor.active().unwrap().is_dirty());

    // 磁盘上的文件被其他程序删除后，自动保存不写回文件，只提示一次如何重新创建；再次失去焦点
    // 不再提示。用户手动保存时仍然新建文件。
    std::fs::remove_file(&b).unwrap();
    let ids: Vec<DocumentId> = app.editor.documents.iter().map(|d| d.id).collect();
    app.auto_save(&ids);
    assert!(!b.exists());
    assert!(app.editor.active().unwrap().is_dirty());
    let notice = app.notice.take().unwrap();
    assert!(notice.contains("已从磁盘上删除"), "{notice}");
    app.auto_save(&ids);
    assert!(app.notice.is_none());
    assert!(app.run_action(crate::keymap::Action::Save));
    assert!(b.exists());

    // 从未保存的新文档不自动保存：该文档没有路径，且自动保存不会弹出「另存为」。
    app.new_file();
    app.run(Command::Insert("new".into()));
    let ids: Vec<DocumentId> = app.editor.documents.iter().map(|d| d.id).collect();
    app.auto_save(&ids);
    assert!(app.editor.active().unwrap().is_dirty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn editorconfig_sets_indentation_and_tidies_on_save() {
    let dir = winc_config::files::canonicalize(sample_folder("editorconfig")).unwrap();
    std::fs::write(
        dir.join(".editorconfig"),
        "root = true\n[*]\nindent_style = space\nindent_size = 2\nend_of_line = lf\n\
         trim_trailing_whitespace = true\ninsert_final_newline = true\n[*.md]\n\
         trim_trailing_whitespace = false\n",
    )
    .unwrap();
    // 文件内容为四格缩进和 CRLF 行尾：缩进采用 `.editorconfig` 的设置，行尾在保存时才统一。
    let path = dir.join("app.py");
    std::fs::write(&path, "def run():\r\n    return 1   \r\n").unwrap();
    let mut app = app_with(&dir, false);
    app.open_path(&path);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.indent(), winc_core::Indent::Spaces(2));
    assert_eq!(app.editor.active_tab_width(), 2);

    // ⌘S：删除行尾空白，行尾统一为 LF，并补上末尾换行符（`insert_final_newline`）。
    // ⌘Z 一次撤销全部清理。
    let end = app.editor.active().unwrap().text().end_pos();
    app.select(end, end);
    app.run(Command::Insert("x = 2  ".into()));
    app.save_active();
    let read = || std::fs::read_to_string(&path).unwrap();
    assert_eq!(read(), "def run():\n    return 1\nx = 2\n");
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.line_ending, winc_core::LineEnding::Lf);
    assert!(!doc.is_dirty());
    app.run(Command::Undo);
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.text_range(0, usize::MAX).unwrap(),
        "def run():\r\n    return 1   \r\nx = 2  "
    );

    // 自动保存：光标所在行可能正在输入，不删除该行的行尾空白。
    app.config.auto_save = Some(true);
    let ids: Vec<DocumentId> = app.editor.documents.iter().map(|d| d.id).collect();
    app.auto_save(&ids);
    assert_eq!(read(), "def run():\n    return 1\nx = 2  \n");

    // Markdown 中行尾两个空格表示换行；`.editorconfig` 对 `*.md` 关闭了
    // `trim_trailing_whitespace`，因此保留。
    let md = dir.join("notes.md");
    std::fs::write(&md, "a  \nb").unwrap();
    app.open_path(&md);
    app.run(Command::Insert("#".into()));
    app.save_active();
    assert_eq!(std::fs::read_to_string(&md).unwrap(), "#a  \nb\n");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn status_bar_changes_indentation_and_line_endings() {
    use winc_core::{Indent, LineEnding};

    let dir = search_folder("status-indent");
    let mut app = app_with(&dir, false);
    app.open_path(&dir.join("src/main.rs"));
    paint(&mut app);
    let status_point = |app: &App, item: StatusItem| {
        let (rect, _) = app.status_hits.iter().find(|(_, i)| *i == item).unwrap();
        (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0)
    };
    fn doc(app: &App) -> &winc_editor::Document {
        app.editor.active().unwrap()
    }

    // 缩进和行尾可点击，悬停时显示为按钮；点击缩进弹出菜单，当前使用的缩进方式打勾，
    // 宽度选项在子菜单中。
    let at = status_point(&app, StatusItem::Indent);
    app.on_mouse_move(at.0, at.1);
    assert_eq!(app.hover, Hover::Status(StatusItem::Indent));
    click(&mut app, at);
    assert_eq!(
        menu_commands(&app)[..2],
        [
            (MenuCommand::IndentUsingSpaces, true),
            (MenuCommand::IndentUsingTabs, true)
        ]
    );
    assert!(app.menu_checked(MenuCommand::IndentUsingSpaces));
    assert!(app.menu_checked(MenuCommand::IndentSize(4)));
    assert!(!app.menu_checked(MenuCommand::IndentSize(2)));

    // 选择宽度 2：之后按 Tab 插入两个空格，状态栏文案同步更新。
    app.run_menu_command(MenuCommand::IndentSize(2));
    assert_eq!(doc(&app).indent(), Indent::Spaces(2));
    assert_eq!(
        indent_label(doc(&app).indent(), 4),
        "空格: 2",
        "状态栏的缩进标签显示空格数"
    );

    // 转换为制表符：已有的缩进全部转换，一次撤销即可还原。
    let at = status_point(&app, StatusItem::Indent);
    click(&mut app, at);
    app.run_menu_command(MenuCommand::ConvertToTabs);
    assert_eq!(doc(&app).indent(), Indent::Tabs);
    assert!(
        doc(&app)
            .text_range(0, usize::MAX)
            .unwrap()
            .contains("\tlet config")
    );
    assert_eq!(indent_label(Indent::Tabs, 2), "制表符长度: 2");
    app.run(Command::Undo);
    assert!(
        doc(&app)
            .text_range(0, usize::MAX)
            .unwrap()
            .contains("    let config")
    );

    // 行尾：当前的 LF 打勾，改为 CRLF；命令面板中的「更改行尾序列」弹出的是同一个菜单。
    paint(&mut app);
    let at = status_point(&app, StatusItem::LineEnding);
    click(&mut app, at);
    assert_eq!(
        menu_commands(&app),
        [
            (MenuCommand::LineEnding(LineEnding::Lf), true),
            (MenuCommand::LineEnding(LineEnding::CrLf), true)
        ]
    );
    assert!(app.menu_checked(MenuCommand::LineEnding(LineEnding::Lf)));
    app.run_menu_command(MenuCommand::LineEnding(LineEnding::CrLf));
    assert_eq!(doc(&app).line_ending, LineEnding::CrLf);
    assert!(doc(&app).is_dirty());
    app.run_action(crate::keymap::Action::ChangeEndOfLine);
    assert!(matches!(
        app.context_menu.as_ref().map(|m| &m.target),
        Some(crate::app::input::context_menu::MenuTarget::LineEnding(_))
    ));
    app.context_menu = None;

    // 仅供查看的输出标签（格式化输出）没有可点击的状态栏项。
    app.editor.show_output("格式化输出", "x\n");
    paint(&mut app);
    assert!(app.status_hits.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_scrollbar_marks_other_occurrences_of_the_selection() {
    let dir = std::env::temp_dir().join(format!("winc-ruler-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 样本共 400 行：第 10、200、390 行各含两处 `total`，第 300 行是 `totally`，其余为 `x`。
    let mut text = String::new();
    for i in 0..400 {
        match i {
            10 | 200 | 390 => text.push_str("let total = total + 1;\n"),
            300 => text.push_str("let totally = 2;\n"),
            _ => text.push_str("x\n"),
        }
    }
    let file = dir.join("a.rs");
    std::fs::write(&file, &text).unwrap();
    let mut app = app_with(&file, false);
    let marks = |app: &App| {
        app.ruler_cache
            .as_ref()
            .and_then(|(_, l)| l.as_deref().map(<[usize]>::to_vec))
    };

    // 只有光标、没有选区：光标停在第 200 行（从 0 起）的 `total` 上也不标。
    let at = app.editor.active().unwrap().text().line_start(200) + 4;
    app.select(at, at);
    paint(&mut app);
    assert_eq!(marks(&app), None);

    // 选中 `total`：标出包含该词的每一行（`totally` 里也有），同一行出现两次也只标一次。
    app.select(at, at + 5);
    paint(&mut app);
    assert_eq!(marks(&app), Some(vec![10, 200, 300, 390]));
    let key = app.ruler_cache.as_ref().unwrap().0.clone();

    // 选区不变、滚动：不重新计算。
    app.editor.active_mut().unwrap().scroll_line = 100.0;
    paint(&mut app);
    assert_eq!(app.ruler_cache.as_ref().unwrap().0, key);

    // 选中的文字只出现一次（第 300 行的 `totally`）：不标记。
    let at = app.editor.active().unwrap().text().line_start(300) + 4;
    app.select(at, at + 7);
    paint(&mut app);
    assert_eq!(marks(&app), None);

    // 文本改变后：重新计算。
    let at = app.editor.active().unwrap().text().line_start(10) + 4;
    app.select(at, at);
    app.run(Command::Insert("total ".into()));
    app.select(at, at + 5);
    paint(&mut app);
    assert_eq!(marks(&app), Some(vec![10, 200, 300, 390]));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bookmarks_jump_list_and_survive_closing_and_restarting() {
    let dir = search_folder("bookmarks");
    let main = dir.join("src/main.rs");
    let mut app = app_with(&main, false);
    let line_of = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.text().line_of(doc.selection.head)
    };
    let go = |app: &mut App, line: usize| {
        let at = app.editor.active().unwrap().text().line_start(line);
        app.select(at, at);
    };

    // 没有书签时无法跳转，状态栏给出提示。
    app.run_action(Action::NextBookmark);
    assert_eq!(app.status_message.as_deref(), Some("此文件中没有书签"));

    // 在第 1、3 行（从 0 起）添加书签；第 2 行切换两次，即添加后又移除。
    go(&mut app, 3);
    app.run_action(Action::ToggleBookmark);
    go(&mut app, 1);
    app.run_action(Action::ToggleBookmark);
    go(&mut app, 2);
    app.run_action(Action::ToggleBookmark);
    app.run_action(Action::ToggleBookmark);
    assert_eq!(app.editor.active().unwrap().bookmark_lines(), [1, 3]);

    // 跳到下一个、上一个书签；越过末尾或开头时回绕到另一端。
    go(&mut app, 0);
    app.run_action(Action::NextBookmark);
    assert_eq!(line_of(&app), 1);
    app.run_action(Action::NextBookmark);
    assert_eq!(line_of(&app), 3);
    app.run_action(Action::NextBookmark);
    assert_eq!(line_of(&app), 1);
    app.run_action(Action::PreviousBookmark);
    assert_eq!(line_of(&app), 3);

    // 列出书签：卡片中有两行，回车跳转到选中的书签。
    app.run_action(Action::ListBookmarks);
    assert!(app.picker_open);
    assert_eq!(app.picker_hits.len(), 2);
    paint(&mut app);
    app.picker_selected = 0;
    app.places_accept();
    assert!(!app.picker_open);
    assert_eq!(line_of(&app), 1);

    // 关闭标签后重新打开：书签仍在。
    app.close_tab(0);
    app.open_path(&main);
    assert_eq!(app.editor.active().unwrap().bookmark_lines(), [1, 3]);

    // 退出后重新打开该目录：书签仍在，未打开文件的书签也保留。
    app.save_session();
    let session = app.state.last_session().cloned().unwrap();
    assert_eq!(session.bookmarks.len(), 1);
    let mut editor = Editor::default();
    assert!(crate::open_session_folder(&mut editor, &session));
    crate::app::restore_at_launch(&mut editor, Some(&session), None);
    assert_eq!(editor.active().unwrap().bookmark_lines(), [1, 3]);

    app.run_action(Action::ClearBookmarks);
    assert!(app.editor.active().unwrap().bookmark_lines().is_empty());
    assert_eq!(app.status_message.as_deref(), Some("已清除 2 个书签"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn other_encodings_can_be_reopened_and_saved_from_the_status_bar() {
    use winc_editor::Encoding;
    use winc_editor::encoding::CHOICES;

    let dir = std::env::temp_dir().join(format!("winc-encoding-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("readme.txt");
    let text = "这是一份用 GBK 写的说明文件，里面都是中文。\n";
    let gbk_encoding = CHOICES
        .iter()
        .find(|(_, label)| label.contains("GBK"))
        .unwrap()
        .0;
    let gbk = gbk_encoding.encode(text).unwrap();
    std::fs::write(&file, &gbk).unwrap();
    let mut app = app_with(&file, false);
    let index = |e: Encoding| CHOICES.iter().position(|(c, _)| *c == e).unwrap();
    let id = app.editor.active().unwrap().id;

    // 检测为简体中文，状态栏显示 GB18030：猜测为 GBK 时按其超集 GB18030 记录，
    // 保存时四字节的字符也能写回；该项可点击。
    paint(&mut app);
    assert_eq!(app.editor.active().unwrap().encoding.label(), "GB18030");
    assert!(
        app.status_hits
            .iter()
            .any(|&(_, i)| i == StatusItem::Encoding)
    );

    // 按 Big5 重新打开：解码结果与原文不同；再按 GBK 重新打开：恢复原文，且文档可编辑。
    let big5 = CHOICES
        .iter()
        .position(|(_, l)| l.contains("Big5"))
        .unwrap();
    app.run_status_command(MenuCommand::ReopenEncoding(big5), id);
    let doc = app.editor.active().unwrap();
    assert_ne!(doc.text().rope().unwrap().to_string(), text);
    let gbk_index = index(gbk_encoding);
    app.run_status_command(MenuCommand::ReopenEncoding(gbk_index), id);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.text().rope().unwrap().to_string(), text);
    assert!(!doc.readonly);

    // 文档有未保存的修改时，重新打开需要先确认；测试中没有确认框，因此文档保持不变。
    app.run(Command::Insert("x".into()));
    app.run_status_command(MenuCommand::ReopenEncoding(gbk_index), id);
    assert!(app.editor.active().unwrap().is_dirty());

    // 以 UTF-8 编码保存：立即写入，文件变为 UTF-8。
    app.run_status_command(MenuCommand::SaveEncoding(index(Encoding::Utf8)), id);
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.encoding, Encoding::Utf8);
    assert!(!doc.is_dirty());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("x{text}"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn csv_columns_line_up_for_clicks_and_moving_up_and_down() {
    let dir = std::env::temp_dir().join(format!("winc-csv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.csv");
    std::fs::write(&file, "a,b,c\n1111111,22,3\nx,\"y, z\",w\n").unwrap();
    let mut app = app_with(&file, false);
    paint(&mut app);
    let tab = app.editor.active_tab_width();
    let caret = |app: &App| {
        let doc = app.editor.active().unwrap();
        doc.text().pos_to_line_col(doc.selection.head)
    };

    // 第二列从第 9 格（等宽单元格，从 0 起）开始：第一列最宽 7 格，再加分隔符 1 格和间隙 1 格。
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.col_to_cell(0, 2, tab), 9);
    assert_eq!(doc.col_to_cell(1, 8, tab), 9);
    // 引号内的逗号不分列：第三列从 `"y, z"` 之后开始。
    assert_eq!(doc.col_to_cell(2, 9, tab), 17);

    // 在第一行的 `b` 上向下移动：到下一行第二列的开头，再向下仍是第二列的开头。
    let at = app.editor.active().unwrap().text().line_start(0) + 2;
    app.select(at, at);
    app.run(Command::MoveDown { extend: false });
    assert_eq!(caret(&app), (1, 8));
    app.run(Command::MoveDown { extend: false });
    assert_eq!(caret(&app), (2, 2));

    // 点击落在对齐产生的空白中时，光标停在距离更近的字符边界上。
    let t = app.layout.text;
    let doc = app.editor.active().unwrap();
    let x = t.x + TEXT_PAD - doc.scroll_x + 8.5 * app.cell;
    let y = t.y + 0.5 * app.fonts.line_height;
    assert_eq!(
        app.pos_at(x, y),
        Some(2),
        "第一行 `a,` 后面的空白，靠右：b 的前面"
    );

    // 修改文本后，列宽随之重新计算。
    let end = app.editor.active().unwrap().text().line_end(0);
    app.select(end, end);
    app.run(Command::Insert("cccccccccccccccc".into()));
    let doc = app.editor.active_mut().unwrap();
    let columns = doc.columns(tab).unwrap();
    assert_eq!(columns.starts.len(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn word_wrap_folds_long_lines_to_the_editor_width() {
    let dir = sample_folder("word-wrap");
    let long = "word ".repeat(80);
    let path = dir.join("long.txt");
    // 后面补 60 行，内容比一屏高，能滚动。
    let filler = "x\n".repeat(60);
    std::fs::write(&path, format!("first\n{long}\nlast\n{filler}")).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    assert_eq!(app.row_count(), 64);

    // 开启自动换行（`⌥Z`）：长行折成多段显示，不再横向滚动，并在状态栏给出提示。
    app.run_action(Action::ToggleWordWrap);
    assert_eq!(app.config.word_wrap, Some(true));
    assert_eq!(app.status_message.as_deref(), Some("已开启自动换行"));
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    let rows = doc.rows_of(1);
    assert!(rows >= 3, "{rows}");
    assert_eq!(app.row_count(), 63 + rows);
    assert_eq!(doc.scroll_x, 0.0);

    // 点击第二段的开头：光标落在该段第一个字符之前。
    let t = app.layout.text;
    let y = t.y + 2.5 * app.fonts.line_height;
    let pos = app.pos_at(t.x + TEXT_PAD, y).unwrap();
    let doc = app.editor.active().unwrap();
    let start = doc.text().line_start(1) + doc.line_breaks(1).starts[0];
    assert_eq!(pos, start);

    // ↓ 按屏幕上的显示行移动：从第一行到长行的第一段，再到第二段。
    app.select(0, 0);
    app.run(winc_editor::Command::MoveDown { extend: false });
    app.run(winc_editor::Command::MoveDown { extend: false });
    let doc = app.editor.active().unwrap();
    assert_eq!(doc.row_at(doc.selection.head), 2);

    // 编辑区变窄（侧栏变宽）：长行断成更多段；滚动位置顶部的文本行保持不变。窄掉文本区的一半：
    // 固定的宽度在某些字体下恰好断出同样的段数。
    app.editor.active_mut().unwrap().scroll_line = 2.0;
    app.panel_width += app.layout.text.w / 2.0;
    paint(&mut app);
    let doc = app.editor.active().unwrap();
    assert!(doc.rows_of(1) > rows);
    assert_eq!(doc.line_of_row(doc.scroll_line as usize), 1);

    // 再按 ⌥Z 关闭：每个文本行只占一个显示行。
    app.run_action(Action::ToggleWordWrap);
    paint(&mut app);
    assert_eq!(app.row_count(), 64);
    assert_eq!(app.status_message.as_deref(), Some("已关闭自动换行"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn triple_click_selects_whole_lines_while_dragging() {
    let dir = sample_folder("triple-click");
    let mut app = app_with(&dir.join("src/main.rs"), false);
    paint(&mut app);

    let l = app.layout;
    let lh = app.fonts.line_height;
    let line = 2;
    let span = app.editor.active().unwrap().text().line_span(line);
    let x = l.text.x + 40.0;
    let y = l.text.y + (line as f32 + 0.5) * lh;
    let click = |app: &mut App| {
        app.on_mouse_move(x, y);
        app.on_mouse_down(MouseButton::Left);
        app.on_mouse_up(MouseButton::Left);
    };

    // 三击选中整行；按住第三下在同一行内移动，选区仍是整行，不会缩到光标处。
    click(&mut app);
    click(&mut app);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x + 20.0, y);
    app.on_mouse_up(MouseButton::Left);
    assert_eq!(app.editor.active().unwrap().selection.range(), span);

    // 按住第三下向上拖到上一行：整段按行选择。
    click(&mut app);
    click(&mut app);
    app.on_mouse_move(x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y - lh);
    app.on_mouse_up(MouseButton::Left);
    let expected = {
        let text = app.editor.active().unwrap().text();
        text.line_span(line - 1).start..text.line_span(line).end
    };
    assert_eq!(app.editor.active().unwrap().selection.range(), expected);

    let _ = std::fs::remove_dir_all(&dir);
}
