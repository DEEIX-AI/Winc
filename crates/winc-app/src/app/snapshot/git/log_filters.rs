//! Git 日志的日期、路径筛选，详情里的所在分支、按目录分组的文件，以及左侧的分支面板。

use super::*;
use crate::app::git::log::{BranchRow, LogTarget};
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

    // 所在分支：选中 `HEAD` 以后停一会儿才查（连续移动时不查），查到的含当前分支 `main`。
    let head = app
        .editor
        .active()
        .unwrap()
        .log
        .as_deref()
        .unwrap()
        .head_row();
    app.select_log_row(head.unwrap());
    let branches = |app: &App| {
        let view = app.editor.active().unwrap().log.as_deref().unwrap();
        view.details.as_ref().and_then(|d| d.branches.clone())
    };
    assert!(branches(&app).is_none());
    app.log_tick(std::time::Instant::now() + std::time::Duration::from_secs(1));
    app.poll_log();
    assert!(branches(&app).is_some_and(|b| b.iter().any(|n| n == "main")));
}

#[test]
fn the_branch_panel_filters_checks_out_and_groups_remote_branches() {
    let Some((mut app, root)) = git_app("git-log-branch-panel", false) else {
        return;
    };
    history_repo(&root, 10);
    let long = "release/a-very-long-branch-name-that-does-not-fit";
    git_in(&root, &["branch", long]);
    // 第二个远程：远程分支按远程名各成一组。
    git_in(
        &root,
        &["remote", "add", "upstream", "https://example.invalid/r.git"],
    );
    git_in(&root, &["update-ref", "refs/remotes/upstream/main", "main"]);
    app.git_refresh();
    show_all_log(&mut app);
    app.layout.editor.w = app.layout.editor.w.max(1200.0);
    paint(&mut app);
    assert!(app.log_parts().branches.w > 0.0);
    let at = |app: &App, name: &str| {
        let rows = app.log_branch_rows();
        rows.iter()
            .position(|r| matches!(r, BranchRow::Branch { name: n, .. } if n == name))
    };

    // 放不下的分支名截短，悬停时提示全名。
    let cut = at(&app, long).unwrap();
    assert!(app.git.log.long_branches.contains(&cut));
    assert!(
        !app.git
            .log
            .long_branches
            .contains(&at(&app, "dev").unwrap())
    );

    // 「全部分支」在最上面，本地分支一组，当前分支在组里最前；每个远程一组，写去掉远程名的名字。
    let rows = app.log_branch_rows();
    assert_eq!(rows[0], BranchRow::All);
    assert_eq!(rows[1], BranchRow::Heading(None));
    let main = BranchRow::Branch {
        name: "main".into(),
        label: "main".into(),
        current: true,
    };
    assert_eq!(rows[2], main);
    for remote in ["origin", "upstream"] {
        let heading = rows
            .iter()
            .position(|r| *r == BranchRow::Heading(Some(remote.into())));
        let row = &rows[heading.unwrap() + 1];
        let want = format!("{remote}/main");
        assert!(
            matches!(row, BranchRow::Branch { name, label, .. } if *name == want && label == "main")
        );
    }

    // 点一个分支只看它的提交，再点一次回到全部。
    let login = at(&app, "feature/login").unwrap();
    app.press_log(LogTarget::BranchRow(login));
    assert_eq!(app.log_branch_filter(), Some("feature/login"));
    app.press_log(LogTarget::BranchRow(login));
    assert_eq!(app.log_branch_filter(), None);

    // 右键是分支选择器里同样的操作菜单；双击签出。
    paint(&mut app);
    let (rect, _) = *app
        .hits
        .log
        .iter()
        .find(|(_, t)| *t == LogTarget::BranchRow(login))
        .unwrap();
    let index = app
        .log_branch_context_target(rect.x + 20.0, rect.y + 4.0)
        .unwrap();
    assert_eq!(app.git.branches[index].name, "feature/login");
    app.checkout_log_branch_row(login);
    app.git_refresh();
    assert!(
        app.git
            .branches
            .iter()
            .any(|b| b.current && b.name == "feature/login")
    );

    // 收起本地分支一组；隐藏面板后列表占回它的宽度。
    app.press_log(LogTarget::BranchRow(1));
    assert!(at(&app, "dev").is_none());
    let list = app.log_parts().list.x;
    app.press_log(LogTarget::BranchesToggle);
    assert_eq!(app.log_parts().branches.w, 0.0);
    assert!(app.log_parts().list.x < list);
}

#[test]
fn log_menus_filter_by_me_copy_a_patch_and_compare_a_file_with_local() {
    let Some((mut app, root)) = git_app("git-log-more-menus", false) else {
        return;
    };
    history_repo(&root, 10);
    git_in(&root, &["config", "user.email", "alice@example.com"]);
    app.git_refresh();
    show_all_log(&mut app);
    paint(&mut app);
    let view = |app: &App| {
        app.editor
            .active()
            .unwrap()
            .log
            .as_deref()
            .unwrap()
            .commits
            .clone()
    };

    // 作者菜单里「我」按仓库配置的邮箱筛选。
    assert_eq!(app.git.my_email, "alice@example.com");
    app.press_log(LogTarget::Author);
    app.run_menu_command(MenuCommand::LogAuthorMe);
    let mine = view(&app);
    assert!(!mine.is_empty() && mine.iter().all(|c| c.email == "alice@example.com"));
    app.press_log(LogTarget::ClearFilters);

    // 复制为补丁：`git format-patch` 的格式，以该提交号开头（中文说明在邮件头里会编码）。
    let commit = view(&app)[0].clone();
    app.open_context_menu(MenuTarget::LogCommit(commit.oid.clone()), (0.0, 0.0));
    app.run_menu_command(MenuCommand::LogCopyPatch);
    app.poll_log();
    let patch = app.clipboard.copied.clone().unwrap_or_default();
    assert!(
        patch.starts_with(&format!("From {} ", commit.oid)),
        "{patch}"
    );

    // 与本地版本比较：左边是该提交里的版本，右边是工作区里的。
    let path = root.join("src/login.rs");
    std::fs::write(&path, "local edit\n").unwrap();
    let row = view(&app)
        .iter()
        .position(|c| c.subject == "登录：失败三次后锁定")
        .unwrap();
    app.select_log_row(row);
    app.open_context_menu(MenuTarget::LogFile(0, path.clone()), (0.0, 0.0));
    app.run_menu_command(MenuCommand::LogCompareWithLocal);
    app.poll_log();
    let doc = app.editor.active().unwrap();
    assert!(
        doc.title
            .as_deref()
            .is_some_and(|t| t.starts_with("login.rs"))
    );
    let diff = doc.diff.as_deref().unwrap();
    assert_eq!(diff.new.text.to_string(), "local edit\n");
}
