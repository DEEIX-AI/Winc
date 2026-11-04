//! 合并冲突的测试：标记行上的按钮、「采用对方」与「两者都保留」、标记为已解决并完成合并。

use super::*;

/// 在 `dir` 中做一次会冲突的合并：`main` 与 `other` 修改了 `src/main.rs` 的同一行。
fn conflicted_repo(dir: &Path) -> bool {
    let text: String = (0..30).map(|i| format!("let value_{i} = {i};\n")).collect();
    std::fs::write(dir.join("src/main.rs"), &text).unwrap();
    if !make_repo(dir) {
        return false;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    // `make_repo` 修改或新增的文件先储藏起来，工作区干净才能合并。
    assert!(git(&["stash", "-u", "-q"]));
    assert!(git(&["switch", "-q", "-c", "other"]));
    std::fs::write(
        dir.join("src/main.rs"),
        text.replace("let value_10 = 10;", "let value_10 = 1000; // other"),
    )
    .unwrap();
    assert!(git(&["commit", "-q", "-am", "other"]));
    assert!(git(&["switch", "-q", "main"]));
    std::fs::write(
        dir.join("src/main.rs"),
        text.replace("let value_10 = 10;", "let value_10 = 100; // main"),
    )
    .unwrap();
    assert!(git(&["commit", "-q", "-am", "main"]));
    assert!(!git(&["merge", "other"]));
    true
}

#[test]
fn conflicts_resolve_in_the_file_and_finish_the_merge() {
    let dir = search_folder("git-merge");
    if !conflicted_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.operation, Some(winc_editor::git::Operation::Merge));
    let index = app.editor.active.unwrap();
    assert_eq!(app.conflicts(index).len(), 1);
    // 合并的提交信息草稿已经填上。
    assert!(app.commit.message.text().contains("other"));

    // 标记行上有按钮；右上角的卡片说还剩一处。
    paint(&mut app);
    use crate::app::git::merge::MergeTarget;
    use winc_editor::git::merge::Pick;
    assert!(
        app.merge
            .hits
            .iter()
            .any(|(_, t)| *t == MergeTarget::Pick(0, Pick::Both))
    );
    assert!(app.merge.card.is_some());
    // 下一处冲突：光标跳到冲突那一行。
    assert!(app.run_action(Action::NextDifference));

    // 采用对方：整段换成对方的那一行，可以撤销。
    app.press_merge(MergeTarget::Pick(0, Pick::Theirs));
    let text = app
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(text.contains("let value_10 = 1000; // other\n"));
    assert!(!text.contains("<<<<<<<") && !text.contains("// main"));
    assert!(app.run_action(Action::Undo));
    assert_eq!(app.conflicts(index).len(), 1);
    app.press_merge(MergeTarget::Pick(0, Pick::Both));
    let text = app
        .editor
        .active()
        .unwrap()
        .text()
        .rope()
        .unwrap()
        .to_string();
    assert!(text.contains("// main\nlet value_10 = 1000; // other\n"));

    // 冲突全部解决后：卡片变成「标记为已解决」；点击后保存并 `git add`。
    paint(&mut app);
    assert!(
        app.merge
            .hits
            .iter()
            .any(|(_, t)| *t == MergeTarget::MarkResolved)
    );
    app.press_merge(MergeTarget::MarkResolved);
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert!(
        status
            .entries
            .iter()
            .all(|e| e.shown() != winc_editor::git::Change::Conflicted)
    );

    app.git_commit();
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.operation, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_with_conflict_markers_are_not_committed() {
    let dir = search_folder("git-merge-guard");
    if !conflicted_repo(&dir) {
        return;
    }
    let mut app = app_with(&dir.join("src/main.rs"), false);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return;
    }
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.unresolved.len(), 1);
    // 冲突的文件单独成组并排在最上方；提交信息上方显示说明条和「中止」。
    let first = app.commit_rows().first().cloned();
    assert!(matches!(
        first,
        Some(CommitRow::Group(CommitGroup::Conflicts))
    ));
    assert!(app.commit_parts().banner.is_some());
    assert!(
        app.commit_hits
            .iter()
            .any(|(_, t)| *t == CommitTarget::Abort)
    );
    // 仍有冲突标记时按钮不可用，按下也不会提交。
    let blocker = app.commit.blocker(&status);
    assert_eq!(blocker.as_deref(), Some("1 个文件还有未解决的冲突"));
    app.git_commit();
    app.git_refresh();
    assert_eq!(
        app.git.status.as_ref().unwrap().operation,
        Some(winc_editor::git::Operation::Merge)
    );

    // 在编辑器中选定一侧但尚未保存时，按下按钮先保存；标记已移除，因此正常提交。
    use crate::app::git::merge::MergeTarget;
    use winc_editor::git::merge::Pick;
    paint(&mut app);
    app.press_merge(MergeTarget::Pick(0, Pick::Theirs));
    app.git_commit();
    app.git_refresh();
    let status = app.git.status.clone().unwrap();
    assert_eq!(status.operation, None);
    assert!(status.unresolved.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 截图：在文件里解决合并冲突，深浅两套配色。
#[test]
#[ignore]
fn shoot_merge_conflict() {
    for dark in [false, true] {
        let dir = search_folder(if dark {
            "git-merge-dark"
        } else {
            "git-merge-light"
        });
        if !conflicted_repo(&dir) {
            return;
        }
        let mut app = app_with(&dir.join("src/main.rs"), dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::NextDifference);
        shoot(&mut app, if dark { "merge-dark" } else { "merge-light" });
        app.run_action(Action::ShowCommit);
        shoot(
            &mut app,
            if dark {
                "merge-commit-dark"
            } else {
                "merge-commit-light"
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
