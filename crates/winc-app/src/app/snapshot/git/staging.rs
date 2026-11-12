//! 暂存区的离屏交互验证：分组、按钮命中、索引差异与键盘导航。

use super::*;

fn index(app: &App, path: &Path) -> usize {
    app.git
        .status
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|e| e.path == path)
        .unwrap()
}

fn press(app: &mut App, target: CommitTarget) {
    paint(app);
    if !app.hits.commit.iter().any(|(_, hit)| *hit == target) {
        let row = match target {
            CommitTarget::StageFile(index) => CommitTarget::File(index),
            CommitTarget::UnstageFile(index) => CommitTarget::StagedFile(index),
            _ => target,
        };
        let rect = app
            .hits
            .commit
            .iter()
            .find(|(_, hit)| *hit == row)
            .unwrap()
            .0;
        app.on_mouse_move(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        paint(app);
    }
    let rect = app
        .hits
        .commit
        .iter()
        .find(|(_, hit)| *hit == target)
        .unwrap()
        .0;
    let point = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    app.on_mouse_move(point.0, point.1);
    paint(app);
    assert_eq!(app.hover, Hover::Commit(target));
    // 每次模拟独立单击，避免快速执行被识别为打开文件的双击。
    app.last_click = None;
    click(app, point);
    paint(app);
}

#[test]
fn staging_buttons_split_versions_preview_index_and_preserve_worktree() {
    let Some((mut app, root)) = git_app("git-staging-buttons", false) else {
        return;
    };
    let path = root.join("src/main.rs");
    app.run_action(Action::ShowCommit);
    let file = index(&app, &path);
    paint(&mut app);
    let visible = |app: &App, target| app.hits.commit.iter().any(|(_, hit)| *hit == target);
    assert!(!visible(&app, CommitTarget::StageFile(file)));
    let row = app
        .hits
        .commit
        .iter()
        .find(|(_, hit)| *hit == CommitTarget::File(file))
        .unwrap()
        .0;
    app.on_mouse_move(row.x + row.w / 2.0, row.y + row.h / 2.0);
    paint(&mut app);
    assert!(visible(&app, CommitTarget::StageFile(file)));
    app.on_mouse_move(0.0, 0.0);
    paint(&mut app);
    assert!(!visible(&app, CommitTarget::StageFile(file)));
    press(&mut app, CommitTarget::File(file));
    app.on_mouse_move(0.0, 0.0);
    paint(&mut app);
    for target in [
        CommitTarget::StageFile(file),
        CommitTarget::OpenFile(file),
        CommitTarget::RevertFile(file),
    ] {
        assert!(visible(&app, target));
    }
    press(&mut app, CommitTarget::StageFile(file));
    let entry = &app.git.status.as_ref().unwrap().entries[index(&app, &path)];
    assert!(entry.staged.is_some() && entry.unstaged.is_none());
    assert!(
        app.commit_rows()
            .contains(&CommitRow::Group(CommitGroup::Staged))
    );
    let staged = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{staged}// working only\n")).unwrap();
    app.git_refresh();
    paint(&mut app);
    let file = index(&app, &path);
    let rows = app.commit_rows();
    for group in [CommitGroup::Staged, CommitGroup::Changes] {
        assert!(rows.iter().any(|r| matches!(r, CommitRow::File { index, group: g, .. } if *index == file && *g == group)));
    }
    assert!(
        !app.hits
            .commit
            .iter()
            .any(|(_, hit)| matches!(hit, CommitTarget::Check(_)))
    );
    press(&mut app, CommitTarget::StagedFile(file));
    app.on_mouse_move(0.0, 0.0);
    paint(&mut app);
    assert!(visible(&app, CommitTarget::UnstageFile(file)));
    assert!(!visible(&app, CommitTarget::StageFile(file)));
    assert!(!visible(&app, CommitTarget::RevertFile(file)));
    let rect = |target| {
        app.hits
            .commit
            .iter()
            .find(|(_, hit)| *hit == target)
            .unwrap()
            .0
    };
    let open = rect(CommitTarget::OpenStagedFile(file));
    let unstage = rect(CommitTarget::UnstageFile(file));
    assert!((unstage.x - open.right() - 2.0).abs() < 0.01);
    let diff = app.editor.active().unwrap().diff.as_ref().unwrap();
    assert_eq!(diff.new.text.to_string(), staged);
    assert!(diff.new.path.is_none());
    assert!(diff.repo_path.is_none());
    app.commit_list_key(&Key::Named(NamedKey::ArrowDown));
    assert_eq!(app.commit.selected_group, CommitGroup::Changes);
    assert_eq!(app.commit.selected.as_deref(), Some(path.as_path()));
    paint(&mut app);
    assert!(visible(&app, CommitTarget::StageFile(file)));
    assert!(!visible(&app, CommitTarget::UnstageFile(file)));
    let diff = app.editor.active().unwrap().diff.as_ref().unwrap();
    assert_eq!(diff.old.text.to_string(), staged);
    assert_eq!(diff.old.title, winc_i18n::t!("git-diff-side-index"));
    assert!(diff.new.text.to_string().ends_with("// working only\n"));
    assert_eq!(diff.stats(), (1, 0));
    app.commit_list_key(&Key::Named(NamedKey::ArrowUp));
    assert_eq!(app.commit.selected_group, CommitGroup::Staged);
    app.commit.message.set_text("only staged");
    app.git_commit();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{staged}// working only\n")
    );
    let entry = &app.git.status.as_ref().unwrap().entries[index(&app, &path)];
    assert!(entry.staged.is_none() && entry.unstaged.is_some());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unstage_button_preserves_disk_and_moves_file_back_to_changes() {
    let Some((mut app, root)) = git_app("git-unstage-button", false) else {
        return;
    };
    let path = root.join("src/main.rs");
    let disk = std::fs::read(&path).unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    let file = index(&app, &path);
    press(&mut app, CommitTarget::UnstageFile(file));
    assert_eq!(std::fs::read(&path).unwrap(), disk);
    assert!(
        !app.commit_rows()
            .contains(&CommitRow::Group(CommitGroup::Staged))
    );
    assert!(
        app.commit_rows()
            .contains(&CommitRow::Group(CommitGroup::Changes))
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn worktree_hunk_revert_and_open_buffer_refresh_use_index() {
    let Some((mut app, root)) = git_app("git-staging-hunk-refresh", false) else {
        return;
    };
    let path = root.join("src/main.rs");
    let staged = "fn main() { staged(); }\n";
    std::fs::write(&path, staged).unwrap();
    git_in(&root, &["add", "src/main.rs"]);
    std::fs::write(&path, "fn main() { working(); }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    let file = index(&app, &path);
    press(&mut app, CommitTarget::File(file));
    let diff = app.editor.active().unwrap().diff.as_ref().unwrap();
    assert_eq!(diff.old.text.to_string(), staged);
    assert_eq!(diff.hunks.len(), 1);
    app.revert_diff_hunk(0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), staged);

    // 已打开的编辑器文档也需要在索引改变时刷新左侧，HEAD 全程不变。
    std::fs::write(&path, "fn main() { next(); }\n").unwrap();
    app.open_path(&path);
    app.git_refresh();
    let file = index(&app, &path);
    app.git_preview_changes(&path);
    paint(&mut app);
    let id = app.editor.active().unwrap().id;
    let oid = app.git.status.as_ref().unwrap().oid.clone();
    press(&mut app, CommitTarget::StageFile(file));
    let diff = app.editor.document(id).unwrap().diff.as_ref().unwrap();
    assert_eq!(diff.old.text.to_string(), "fn main() { next(); }\n");
    assert!(diff.is_identical());
    assert_eq!(app.git.status.as_ref().unwrap().oid, oid);
    let file = index(&app, &path);
    press(&mut app, CommitTarget::UnstageFile(file));
    let diff = app.editor.document(id).unwrap().diff.as_ref().unwrap();
    assert!(!diff.is_identical());
    assert_eq!(diff.new.text.to_string(), "fn main() { next(); }\n");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rollback_file_keeps_staged_edits_additions_and_renames() {
    let Some((mut app, root)) = git_app("git-staging-rollback", false) else {
        return;
    };
    app.trash = delete_for_test;
    let main = root.join("src/main.rs");
    let added = root.join("src/new.rs");
    git_in(&root, &["add", "src/main.rs", "src/new.rs"]);
    git_in(&root, &["mv", "src/config.rs", "src/renamed.rs"]);
    let renamed = root.join("src/renamed.rs");
    let paths = [main, added, renamed];
    let staged: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    for path in &paths {
        std::fs::write(path, "unstaged\n").unwrap();
    }
    // 新增文件已暂存后在磁盘删除，也应从索引恢复，而不是从索引移除。
    std::fs::remove_file(&paths[1]).unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    for (path, expected) in paths.iter().zip(&staged) {
        let file = index(&app, path);
        press(&mut app, CommitTarget::File(file));
        press(&mut app, CommitTarget::RevertFile(file));
        assert_eq!(&std::fs::read(path).unwrap(), expected);
        let entry = &app.git.status.as_ref().unwrap().entries[index(&app, path)];
        assert!(entry.staged.is_some());
        assert!(entry.unstaged.is_none());
        let file = index(&app, path);
        press(&mut app, CommitTarget::StagedFile(file));
        let diff = app.editor.active().unwrap().diff.as_ref().unwrap();
        assert_eq!(diff.new.text.to_string().as_bytes(), expected);
    }
    // 暂存区的键盘或菜单入口也不能撤销仅暂存的内容。
    app.git_rollback(paths.to_vec());
    assert!(!root.join("src/config.rs").exists());
    for (path, expected) in paths.iter().zip(&staged) {
        assert_eq!(&std::fs::read(path).unwrap(), expected);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
#[ignore = "截图，手动运行"]
fn shoot_git_staging() {
    for dark in [false, true] {
        let tag = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-staging-{tag}"), false, dark) else {
            return;
        };
        git_in(&root, &["add", "src/main.rs"]);
        std::fs::write(
            root.join("src/main.rs"),
            "fn main() {\n    println!(\"working tree\");\n}\n",
        )
        .unwrap();
        app.git_refresh();
        app.run_action(Action::ShowCommit);
        app.commit.message.set_text("增加暂存区和逐文件暂存按钮");
        shoot(&mut app, &format!("git-staging-{tag}"));
        let file = index(&app, &root.join("src/main.rs"));
        press(&mut app, CommitTarget::File(file));
        app.on_mouse_move(0.0, 0.0);
        shoot(&mut app, &format!("git-staging-selected-{tag}"));
        press(&mut app, CommitTarget::StagedFile(file));
        app.on_mouse_move(0.0, 0.0);
        shoot(&mut app, &format!("git-staging-index-selected-{tag}"));
        let _ = std::fs::remove_dir_all(root);
    }
}
