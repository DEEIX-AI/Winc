//! Git 日志的日期、路径筛选，详情里的所在分支、按目录分组的文件，以及左侧的分支面板。

use super::*;
use crate::app::git::log::{BranchRow, LogFileRow, LogTarget, log_file_rows};
use crate::app::input::context_menu::{MenuCommand, MenuTarget};

/// 在仓库 `root` 里数 `git log --all` 加上 `args` 列出的提交。
fn count(root: &Path, args: &[&str]) -> usize {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["log", "--all", "--exclude=refs/stash", "--oneline"])
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count()
}

#[test]
fn log_filters_by_date_and_path_and_lists_the_containing_branches() {
    let Some((mut app, root)) = git_app("git-log-more-filters", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let listed = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .commits
            .len()
    };
    let all = listed(&app);
    assert_eq!(all, count(&root, &[]));

    // 日期：最近 7 天，与 `git log --since` 一致；改回全部日期。
    app.press_log(LogTarget::Date);
    app.run_menu_command(MenuCommand::LogSince(7));
    let recent = count(&root, &["--since=7.days.ago"]);
    assert!(recent > 0 && recent < all);
    assert_eq!(listed(&app), recent);
    app.open_context_menu(MenuTarget::LogDate, (0.0, 0.0));
    app.run_menu_command(MenuCommand::LogSince(0));
    assert_eq!(listed(&app), all);

    // 路径：菜单里列出仓库根目录下的目录；输入的路径也算上，再选一次去掉。
    paint(&mut app);
    app.press_log(LogTarget::Path);
    assert!(app.git.log.paths.iter().any(|p| p == "src"));
    app.overlays.context_menu = None;
    app.toggle_log_path("src/login.rs".into());
    assert_eq!(listed(&app), count(&root, &["--", "src/login.rs"]));
    assert!(listed(&app) < all);
    app.toggle_log_path("src/login.rs".into());
    assert_eq!(listed(&app), all);

    // 所在分支：选中 `HEAD` 以后查到，含当前分支 `main`。
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    let branches = view.details.as_ref().and_then(|d| d.branches.clone());
    assert!(branches.is_some_and(|b| b.iter().any(|n| n == "main")));
}

#[test]
fn changed_files_group_by_directory_in_display_order() {
    use winc_editor::git::{Change, CommitFile};
    let root = Path::new("/r");
    let file = |path: &str| CommitFile {
        path: root.join(path),
        from: None,
        change: Change::Modified,
        lines: Some((1, 0)),
    };
    let files = [
        file("b/x.rs"),
        file("a.rs"),
        file("b/c/y.rs"),
        file("b/z.rs"),
    ];
    let rows = log_file_rows(Some(root), &files, false);
    use LogFileRow::{Dir, File};
    assert_eq!(
        rows,
        [
            File(1),
            Dir("b".into()),
            File(0),
            File(3),
            Dir("b/c".into()),
            File(2),
        ]
    );
    let flat = log_file_rows(Some(root), &files, true);
    assert_eq!(flat, [File(0), File(1), File(2), File(3)]);
}

#[test]
fn the_branch_panel_filters_the_log_by_a_branch() {
    let Some((mut app, root)) = git_app("git-log-branch-panel", false) else {
        return;
    };
    history_repo(&root, 10);
    app.git_refresh();
    show_all_log(&mut app);
    app.layout.editor.w = app.layout.editor.w.max(1200.0);
    paint(&mut app);
    assert!(app.log_parts().branches.w > 0.0);

    // 「全部分支」在最上面，本地分支一组，当前分支在组里最前、打着 ✓。
    let rows = app.log_branch_rows();
    assert_eq!(rows[0], BranchRow::All);
    assert_eq!(rows[1], BranchRow::Heading(false));
    assert_eq!(rows[2], BranchRow::Branch("main".into(), true));

    // 点一个分支只看它的提交，再点一次回到全部。
    let at = (rows.iter())
        .position(|r| matches!(r, BranchRow::Branch(n, false) if n == "feature/login"))
        .unwrap();
    app.press_log(LogTarget::BranchRow(at));
    assert_eq!(app.log_branch_filter(), Some("feature/login"));
    app.press_log(LogTarget::BranchRow(at));
    assert_eq!(app.log_branch_filter(), None);

    // 收起本地分支一组；隐藏面板后列表占回它的宽度。
    app.press_log(LogTarget::BranchRow(1));
    assert!(!(app.log_branch_rows()).contains(&BranchRow::Branch("main".into(), true)));
    let list = app.log_parts().list.x;
    app.press_log(LogTarget::BranchesToggle);
    assert_eq!(app.log_parts().branches.w, 0.0);
    assert!(app.log_parts().list.x < list);
}
