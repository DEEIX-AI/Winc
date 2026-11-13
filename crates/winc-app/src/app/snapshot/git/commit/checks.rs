//! 提交视图中勾选与暂存的测试：勾选框读写暂存区，提交取暂存区的内容。

use super::*;

/// 返回 `root` 中 `HEAD` 里的 `src/main.rs`。
fn head_main(root: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", "HEAD:src/main.rs"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// 返回 `root` 的暂存区相对 `HEAD` 的差异（`git diff --cached`）。
fn staged_diff(root: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--cached"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// 返回文件 `path` 的勾选框状态（见 [`CommitState::file_state`]）。
fn file_state(app: &App, path: &Path) -> Option<bool> {
    let status = app.git.status.as_ref().unwrap();
    let entry = status.entries.iter().find(|e| e.path == path).unwrap();
    app.git.commit.file_state(entry)
}

/// 返回当前差异标签中各段改动的暂存状态。
fn picks(app: &App) -> Vec<winc_core::diff::Pick> {
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let entry = app.diff_entry(view).unwrap();
    app.git.commit.hunk_inclusions(entry, view)
}

#[test]
fn only_the_staged_hunks_are_committed() {
    use winc_core::diff::Pick;
    let dir = search_folder("git-partial");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    app.git_preview_changes(&file);
    paint(&mut app);
    // 两处改动各有一个勾选框，都没有暂存；勾上第一处（`value_5` 改成 500），文件变成「部分」。
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::Include(1))
    );
    assert_eq!(picks(&app), [Pick::Skip, Pick::Skip]);
    assert_eq!(file_state(&app, &file), None);
    app.press_diff(DiffTarget::Include(0));
    assert_eq!(picks(&app), [Pick::Apply, Pick::Skip]);
    assert_eq!(file_state(&app, &file), Some(false));
    // 暂存区里只有这一处：与在终端里看到的一致。
    let staged = staged_diff(&root);
    assert!(staged.contains("+let value_5 = 500;"), "{staged}");
    assert!(!staged.contains("-let value_50 = 50;"), "{staged}");

    // 暂存之后又修改了别处：新的修改不在暂存区里，不进入提交。
    let disk = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        disk.replace(
            "let value_49 = 49;\n",
            "let value_49 = 49;\nlet value_50 = 5000;\n",
        ),
    )
    .unwrap();

    app.git.commit.message.set_text("only value_5");
    app.git_commit();
    let head = head_main(&root);
    assert!(head.contains("let value_5 = 500;"));
    assert!(head.contains("let value_50 = 50;") && !head.contains("5000"));
    // 没提交的那处还在工作区里，文件仍然有改动，不再有暂存的内容。
    let disk = std::fs::read_to_string(&file).unwrap();
    assert!(disk.contains("let value_50 = 5000;"));
    app.git_refresh();
    assert_eq!(file_state(&app, &file), None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_checkbox_stages_files_and_the_commit_takes_the_index() {
    let dir = search_folder("git-commit");
    if !make_repo(&dir) {
        return;
    }
    git_in(&dir, &["add", "src/new.rs"]);
    let mut app = app_with(&dir, false);
    // 默认显示为列表；本测试检查按目录排列的树形视图。
    app.git.commit.set_flat(false);
    // 模块关闭时：侧栏没有 Git 按钮，⌃⇧G 不响应。
    assert_eq!(app.sidebar_views().len(), 2);
    assert!(!app.run_action(Action::ShowCommit));

    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    assert_eq!(app.sidebar_views().last(), Some(&SidebarView::Commit));
    assert!(app.run_action(Action::ShowCommit));
    assert_eq!(app.panels.sidebar.view, SidebarView::Commit);
    assert_eq!(app.focus, Focus::CommitMessage);
    paint(&mut app);

    // 「更改」一组里是改过的 `src/main.rs` 和在终端里暂存的新文件 `src/new.rs`，都在 `src` 目录
    // 下面；勾选框显示暂存区：只有 `src/new.rs` 勾上，组的勾选框是「部分」。
    let root = app.git.root.clone().unwrap();
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let rows = app.commit_rows();
    let src = |group| CommitRow::Dir {
        group,
        dir: PathBuf::from("src"),
        label: "src".into(),
        depth: 0,
    };
    assert_eq!(
        rows[..],
        [
            CommitRow::Group(CommitGroup::Changes),
            src(CommitGroup::Changes),
            CommitRow::File { index: 0, depth: 1 },
            CommitRow::File { index: 1, depth: 1 },
        ]
    );
    assert_eq!(file_state(&app, &main), None);
    assert_eq!(file_state(&app, &new), Some(true));
    let group = |app: &App| {
        let status = app.git.status.as_ref().unwrap();
        app.git.commit.group_state(status, CommitGroup::Changes)
    };
    assert_eq!(group(&app), Some(false));
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));

    // 点 `src/main.rs` 的勾选框把它暂存；点它那一行打开差异。
    let target =
        |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let check = target(&app, CommitTarget::Check(0));
    click(&mut app, (check.x + 4.0, check.y + 4.0));
    assert_eq!(file_state(&app, &main), Some(true));
    assert_eq!(group(&app), Some(true));
    assert!(staged_diff(&root).contains("src/main.rs"));
    paint(&mut app);
    let row = target(&app, CommitTarget::File(0));
    click(&mut app, (row.x + row.w / 2.0, row.y + 4.0));
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("main.rs · 差异")
    );

    // 键盘：焦点在列表里，↓ 选下一个文件，差异在同一个预览标签里换内容；空格切换暂存。
    let tabs = app.editor.documents.len();
    let key = |app: &mut App, key: NamedKey| app.commit_list_key(&Key::Named(key));
    assert_eq!(app.focus, Focus::CommitList);
    key(&mut app, NamedKey::ArrowDown);
    assert_eq!(app.git.commit.selected.as_deref(), Some(new.as_path()));
    assert_eq!(app.editor.documents.len(), tabs);
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("new.rs · 差异")
    );
    // 取消暂存新文件：它恢复为未跟踪；再按一次重新暂存。
    key(&mut app, NamedKey::Space);
    assert_eq!(file_state(&app, &new), None);
    let status = app.git.status.clone().unwrap();
    assert!(
        status
            .entries
            .iter()
            .any(|e| e.path == new && e.untracked())
    );
    key(&mut app, NamedKey::Space);
    assert_eq!(file_state(&app, &new), Some(true));

    // 没写提交信息：不提交。
    app.git_commit();
    assert_eq!(app.git.status.as_ref().unwrap().entries.len(), 2);

    // 只提交 `src/new.rs`：先取消暂存 `src/main.rs`。
    app.stage_paths(std::slice::from_ref(&main), false);
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));
    app.focus = Focus::CommitMessage;
    // 提交信息可以多行：回车换行，⌘↩ 才提交。
    app.edit_field(|f| f.insert("加上 new.rs"));
    app.commit_message_key(&Key::Named(NamedKey::Enter), false, false);
    app.edit_field(|f| f.insert("正文"));
    assert_eq!(app.git.commit.message.text(), "加上 new.rs\n正文");
    app.commit_message_key(&Key::Named(NamedKey::Enter), true, false);
    assert!(!app.git.commit.busy);
    assert_eq!(app.git.commit.message.text(), "");
    let status = app.git.status.clone().unwrap();
    let left: Vec<_> = status.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(left, std::slice::from_ref(&main));
    assert_eq!(file_state(&app, &main), None);

    // 修改上一次提交：信息填进输入框；只取暂存区的内容，没有暂存时只改提交信息。
    app.run_commit_button_command(crate::app::input::context_menu::MenuCommand::CommitAmend);
    assert_eq!(app.git.commit.message.text(), "加上 new.rs\n正文");
    app.git.commit.message.set_text("加上 new.rs 和 main.rs");
    app.stage_paths(std::slice::from_ref(&main), true);
    app.git_commit();
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());
    paint(&mut app);

    // 回滚：组里没有勾上的文件时回滚整组；未跟踪的文件在另一组，保留。
    let before = std::fs::read_to_string(&main).unwrap();
    std::fs::write(&main, "changed\n").unwrap();
    std::fs::write(root.join("scratch.txt"), "x").unwrap();
    app.git_refresh();
    app.press_commit(CommitTarget::GroupRevert(CommitGroup::Changes));
    assert_eq!(std::fs::read_to_string(&main).unwrap(), before);
    assert!(root.join("scratch.txt").exists());
    // 暂存以后是新增的文件：回滚时移出暂存区并移到废纸篓（测试中直接删除）。
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    app.press_commit(CommitTarget::GroupRevert(CommitGroup::Changes));
    assert!(!root.join("scratch.txt").exists());
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn committing_everything_saves_unsaved_edits_first() {
    let Some((mut app, root)) = git_app("git-commit-dirty", false) else {
        return;
    };
    app.open_path(&root.join("src/main.rs"));
    app.run(winc_editor::Command::Insert("// 还没保存\n".into()));
    assert!(app.editor.active().unwrap().is_dirty());
    app.run_action(Action::ShowCommit);
    app.git.commit.message.set_text("带上没保存的修改");
    app.git_commit();
    // 没有暂存任何内容时提交全部已跟踪文件的改动：提交的是编辑器中看到的内容，不是磁盘上的
    // 旧内容。
    assert!(!app.editor.active().unwrap().is_dirty());
    assert!(head_main(&root).contains("// 还没保存"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn untracked_files_stay_out_of_the_commit_until_staged() {
    let Some((mut app, root)) = git_app("git-commit-untracked", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let status = app.git.status.clone().unwrap();

    // 未跟踪的文件单独成组，默认显示但不勾；没有暂存任何内容时「提交全部」不含未跟踪的文件。
    assert!(!app.git.commit.group_collapsed(CommitGroup::Untracked));
    assert_eq!(file_state(&app, &new), None);
    assert_eq!(
        app.git.commit.group_state(&status, CommitGroup::Untracked),
        None
    );
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);
    assert_eq!(app.commit_scope(), Some(CommitScope::All(1)));

    // 收起和展开不改变勾选；收起时切换文件跳过这一组。
    app.press_commit(CommitTarget::Group(CommitGroup::Untracked));
    assert_eq!(app.commit_files(), std::slice::from_ref(&main));
    assert_eq!(file_state(&app, &new), None);
    app.press_commit(CommitTarget::Group(CommitGroup::Untracked));
    assert_eq!(app.commit_files(), [main.clone(), new.clone()]);

    // 勾上即暂存：成为新增的文件，移到「更改」一组；这时只提交暂存区里的它。
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    assert_eq!(file_state(&app, &new), Some(true));
    let status = app.git.status.clone().unwrap();
    assert!(status.entries.iter().all(|e| !e.untracked()));
    assert_eq!(app.commit_scope(), Some(CommitScope::Staged(1)));
    app.stage_paths(std::slice::from_ref(&new), false);
    assert_eq!(file_state(&app, &new), None);

    // 没有暂存时提交只包含已跟踪的改动，未跟踪的文件仍未跟踪。
    app.git.commit.message.set_text("只提交改动");
    app.git_commit();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].path, new);
    assert!(status.entries[0].untracked());

    // 只剩未跟踪的文件时没有可以提交的内容；暂存以后随提交加入仓库。
    assert_eq!(app.commit_scope(), None);
    app.git.commit.message.set_text("加入新文件");
    assert!(app.git.commit.blocker(&status).is_some());
    app.press_commit(CommitTarget::GroupCheck(CommitGroup::Untracked));
    app.git_commit();
    assert!(app.git.status.as_ref().unwrap().entries.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_the_staged_lines_are_committed() {
    let dir = search_folder("git-partial-lines");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    let disk = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        disk.replace(
            "let value_5 = 500;\n",
            "let value_5 = 500;\nlet added_a = 1;\nlet added_b = 2;\n",
        ),
    )
    .unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);

    // 暂存 `added_b` 所在的那处改动，再把 `added_b` 这一行取消暂存；另一处改动不暂存。
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    let hunk = view
        .hunks
        .iter()
        .position(|h| h.new.contains(&line))
        .unwrap();
    app.press_diff(DiffTarget::Include(hunk));
    assert_eq!(picks(&app)[hunk], winc_core::diff::Pick::Apply);
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    paint(&mut app);
    assert!(
        app.hits
            .diff
            .iter()
            .any(|(_, t)| *t == DiffTarget::IncludeLine(RIGHT, line))
    );
    assert!(matches!(picks(&app)[hunk], winc_core::diff::Pick::Lines(_)));
    assert_eq!(file_state(&app, &file), Some(false));
    let staged = staged_diff(&root);
    assert!(staged.contains("+let added_a = 1;") && !staged.contains("added_b"));

    app.git.commit.message.set_text("only added_a");
    app.git_commit();
    let head = head_main(&root);
    assert!(head.contains("let added_a = 1;"), "{head}");
    assert!(!head.contains("added_b"), "{head}");
    assert!(head.contains("let value_50 = 50;"), "{head}");
    // 没提交的行还在工作区里。
    assert!(std::fs::read_to_string(&file).unwrap().contains("added_b"));

    // 再暂存这一行：这处改动全部暂存。
    app.git_preview_changes(&file);
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let line = text.lines().position(|l| l == "let added_b = 2;").unwrap();
    let hunk = view
        .hunks
        .iter()
        .position(|h| h.new.contains(&line))
        .unwrap();
    app.press_diff(DiffTarget::IncludeLine(RIGHT, line));
    assert_eq!(picks(&app)[hunk], winc_core::diff::Pick::Apply);
    assert!(staged_diff(&root).contains("+let added_b = 2;"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 暂存之后又修改的文件变为「部分」：改过的那处显示为部分暂存，新的改动未暂存，提交取暂存时
/// 的内容。
#[test]
fn a_file_edited_after_staging_is_partly_staged() {
    use winc_core::diff::Pick;
    let Some((mut app, root)) = git_app("git-stage-then-edit", true) else {
        return;
    };
    let file = root.join("src/main.rs");
    app.run_action(Action::ShowCommit);
    app.stage_paths(std::slice::from_ref(&file), true);
    assert_eq!(file_state(&app, &file), Some(true));

    let disk = std::fs::read_to_string(&file).unwrap();
    let edited = disk
        .replace("let value_5 = 500;", "let value_5 = 555;")
        .replace("let value_30 = 30;", "let value_30 = 3000;");
    std::fs::write(&file, edited).unwrap();
    app.git_refresh();
    assert_eq!(file_state(&app, &file), Some(false));

    // 三处改动：`value_5` 暂存后又改过，`value_30` 是新的改动，删掉的 `value_50` 已暂存。
    app.git_preview_changes(&file);
    paint(&mut app);
    let now = picks(&app);
    assert!(matches!(now[0], Pick::Lines(_)), "{now:?}");
    assert_eq!(now[1..], [Pick::Skip, Pick::Apply]);

    app.git.commit.message.set_text("暂存时的内容");
    app.git_commit();
    let head = head_main(&root);
    assert!(head.contains("let value_5 = 500;") && head.contains("let value_30 = 30;"));
    assert!(!head.contains("let value_50 = 50;"));
    assert_eq!(file_state(&app, &file), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// 两处改动内容相同（加入了同样的一行）时各自暂存：暂存其中一处只有这一处进入暂存区，再点
/// 一次取消；提交时也只有暂存的那一处。
#[test]
fn identical_changes_are_staged_one_by_one() {
    use winc_core::diff::Pick;
    let dir = search_folder("git-identical-hunks");
    if !long_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir, false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let root = app.git.root.clone().unwrap();
    let file = root.join("src/main.rs");
    let disk = std::fs::read_to_string(&file).unwrap();
    let disk = disk
        .replace(
            "let value_20 = 20;\n",
            "let value_20 = 20;\nlet same = 1;\n",
        )
        .replace(
            "let value_40 = 40;\n",
            "let value_40 = 40;\nlet same = 1;\n",
        );
    std::fs::write(&file, &disk).unwrap();
    app.git_refresh();
    app.git_preview_changes(&file);
    paint(&mut app);
    let view = app.editor.active().unwrap().diff.as_deref().unwrap();
    let same: Vec<usize> = (0..view.hunks.len())
        .filter(|&h| (0..view.hunks.len()).any(|o| o != h && view.keys[o] == view.keys[h]))
        .collect();
    assert_eq!(same.len(), 2, "two changes with the same content");
    let (first, second) = (same[0], same[1]);

    // 暂存第一处：只有它暂存；再点一次取消。
    app.press_diff(DiffTarget::Include(first));
    let now = picks(&app);
    assert_eq!((&now[first], &now[second]), (&Pick::Apply, &Pick::Skip));
    app.press_diff(DiffTarget::Include(first));
    assert!(picks(&app).iter().all(|p| *p == Pick::Skip));
    assert_eq!(staged_diff(&root), "");

    // 暂存第二处后提交：提交里只有第二处加的那一行。
    app.press_diff(DiffTarget::Include(second));
    let now = picks(&app);
    assert_eq!((&now[first], &now[second]), (&Pick::Skip, &Pick::Apply));
    app.git.commit.message.set_text("only the second");
    app.git_commit();
    let head = head_main(&root);
    assert_eq!(head.matches("let same = 1;").count(), 1, "{head}");
    assert!(head.contains("let value_40 = 40;\nlet same = 1;"), "{head}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文件树和标签的右键菜单对有更改的文件提供暂存、取消暂存与回滚；没有更改的文件没有这几项。
#[test]
fn the_tree_and_tab_menus_stage_and_roll_back_a_changed_file() {
    use crate::app::input::context_menu::MenuCommand::{CommitCheck, FileHistory, GitRollbackFile};
    use crate::app::input::context_menu::MenuTarget;
    let Some((mut app, root)) = git_app("git-tree-menu-stage", false) else {
        return;
    };
    let file = root.join("Cargo.toml");
    let before = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("{before}# changed\n")).unwrap();
    app.git_refresh();
    let commands = |app: &mut App, target: MenuTarget| -> Vec<_> {
        app.open_context_menu(target, (0.0, 0.0));
        menu_commands(app).into_iter().map(|(c, _)| c).collect()
    };

    let listed = commands(&mut app, MenuTarget::Tree(Some(file.clone())));
    assert!(
        listed.ends_with(&[FileHistory, CommitCheck(true), GitRollbackFile]),
        "{listed:?}"
    );
    app.run_menu_command(CommitCheck(true));
    assert_eq!(file_state(&app, &file), Some(true));

    // 已全部暂存的文件只有「取消暂存」；标签的菜单相同。
    app.open_path(&file);
    let id = app.editor.active().unwrap().id;
    let listed = commands(&mut app, MenuTarget::Tab(id));
    assert!(
        listed.ends_with(&[FileHistory, CommitCheck(false), GitRollbackFile]),
        "{listed:?}"
    );
    app.run_menu_command(CommitCheck(false));
    assert_eq!(file_state(&app, &file), None);

    // 回滚恢复为上次提交的内容，之后该文件没有更改，菜单里也没有这几项。
    let _ = commands(&mut app, MenuTarget::Tree(Some(file.clone())));
    app.run_menu_command(GitRollbackFile);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    let listed = commands(&mut app, MenuTarget::Tree(Some(file)));
    assert!(listed.ends_with(&[FileHistory]), "{listed:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交视图里文件的右键菜单：未跟踪的文件可以加入 `.gitignore`，任何文件都可以单独储藏。
#[test]
fn a_changed_file_can_be_ignored_or_stashed_from_its_menu() {
    use crate::app::git::commit::IgnoreKind;
    use crate::app::input::context_menu::MenuCommand::{GitIgnore, GitStashFiles};
    use crate::app::input::context_menu::MenuTarget;
    let Some((mut app, root)) = git_app("git-ignore-and-stash", false) else {
        return;
    };
    let (main, new) = (root.join("src/main.rs"), root.join("src/new.rs"));
    let commands = |app: &mut App, path: &Path| -> Vec<_> {
        app.open_context_menu(MenuTarget::CommitFile(path.to_path_buf()), (0.0, 0.0));
        menu_commands(app).into_iter().map(|(c, _)| c).collect()
    };

    // 已跟踪的文件没有「加入 .gitignore」；未跟踪的文件有三种范围。
    let listed = commands(&mut app, &main);
    assert!(listed.contains(&GitStashFiles));
    assert!(
        !listed.iter().any(|c| matches!(c, GitIgnore(_))),
        "{listed:?}"
    );
    let listed = commands(&mut app, &new);
    for kind in [IgnoreKind::File, IgnoreKind::Extension, IgnoreKind::Folder] {
        assert!(listed.contains(&GitIgnore(kind)), "{kind:?} {listed:?}");
    }

    // 忽略这一个文件：写入仓库根目录的 `.gitignore`，它不再列在更改中；重复忽略不重复写入。
    app.run_menu_command(GitIgnore(IgnoreKind::File));
    let ignore = root.join(".gitignore");
    assert_eq!(std::fs::read_to_string(&ignore).unwrap(), "/src/new.rs\n");
    let listed_paths = |app: &App| -> Vec<PathBuf> {
        let status = app.git.status.as_ref().unwrap();
        status.entries.iter().map(|e| e.path.clone()).collect()
    };
    assert_eq!(listed_paths(&app), [main.clone(), ignore.clone()]);
    app.git_ignore(&new, IgnoreKind::File);
    assert_eq!(std::fs::read_to_string(&ignore).unwrap(), "/src/new.rs\n");

    // 只储藏 `src/main.rs`：它恢复为上次提交的内容，`.gitignore` 仍在更改中。
    let changed = std::fs::read_to_string(&main).unwrap();
    let _ = commands(&mut app, &main);
    app.run_menu_command(GitStashFiles);
    assert_eq!(app.focus, Focus::Prompt);
    app.edit_field(|f| f.insert("只有 main"));
    app.submit_prompt();
    assert_ne!(std::fs::read_to_string(&main).unwrap(), changed);
    assert_eq!(listed_paths(&app), std::slice::from_ref(&ignore));
    assert_eq!(app.git.stashes.len(), 1);
    assert!(app.git.stashes[0].message.contains("只有 main"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 提交按钮的菜单：签署按仓库记住，跳过钩子只对下一次提交有效。
#[test]
fn the_commit_menu_signs_off_and_skips_hooks_once() {
    use crate::app::input::context_menu::MenuCommand::{CommitNoVerify, CommitSignOff};
    use crate::app::input::context_menu::MenuEntry;
    let Some((mut app, root)) = git_app("git-commit-options", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    let menu = app.commit_button_menu();
    for command in [CommitSignOff, CommitNoVerify] {
        assert!(
            menu.contains(&MenuEntry::Item(command, true)),
            "{command:?}"
        );
        assert!(!app.menu_checked(command));
    }
    app.run_commit_button_command(CommitSignOff);
    app.run_commit_button_command(CommitNoVerify);
    assert!(app.menu_checked(CommitSignOff) && app.menu_checked(CommitNoVerify));
    assert_eq!(app.state.sign_off_repos, std::slice::from_ref(&root));

    app.git.commit.message.set_text("带签署的提交");
    app.git_commit();
    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "-1", "--format=%B"])
        .output()
        .unwrap();
    let message = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(message.contains("Signed-off-by: t <t@t>"), "{message}");
    // 提交成功后「跳过钩子」关闭，签署仍然开着；再点一次关闭签署。
    assert!(!app.menu_checked(CommitNoVerify) && app.menu_checked(CommitSignOff));
    app.run_commit_button_command(CommitSignOff);
    assert!(app.state.sign_off_repos.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
