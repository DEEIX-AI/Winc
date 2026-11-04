//! 分支菜单与新建分支对话框的测试：新建、切换、推送、删除，以及远程分支与提交并推送。

use super::*;

#[test]
fn branch_menu_creates_switches_pushes_and_deletes() {
    use crate::app::git::branch::{BranchOp, branch_menu};
    use crate::app::input::context_menu::{MenuCommand, MenuEntry};

    let Some((mut app, root)) = git_app("git-branches", false) else {
        return;
    };
    // 先把工作区的改动提交，保证切换分支时没有未提交内容。
    git_in(&root, &["add", "."]);
    git_in(&root, &["commit", "-q", "-m", "second"]);
    app.git_refresh();
    let index_of = |app: &App, name: &str| {
        app.git
            .branches
            .iter()
            .position(|b| b.name == name)
            .unwrap()
    };
    let head = |app: &App| match app.git.status.as_ref().and_then(|s| s.head.clone()) {
        Some(Head::Branch(name)) => name,
        other => panic!("{other:?}"),
    };

    // 新建分支：对话框里输入名称，回车创建并签出。空格这类字符无法输入。
    app.run_branch_command(MenuCommand::GitNewBranch);
    assert_eq!(app.focus, Focus::Dialog);
    paint(&mut app);
    app.edit_field(|f| f.insert("feature x"));
    let dialog = app.form_dialog.as_ref().unwrap();
    assert_eq!(dialog.name.text(), "featurex");
    assert_eq!(app.form_plan(dialog).base, "main（当前分支）");
    app.submit_form_dialog();
    assert!(app.form_dialog.is_none());
    assert!(app.branch.running.is_none());
    assert_eq!(head(&app), "featurex");

    // 分支重名时不能创建：原因显示在输入框里，对话框保留；取消「创建后签出」时只创建、不切换。
    app.run_branch_command(MenuCommand::GitNewBranch);
    app.edit_field(|f| f.insert("main"));
    let plan = app.form_plan(app.form_dialog.as_ref().unwrap());
    assert!(!plan.enabled);
    assert_eq!(plan.problem.as_deref(), Some("已存在同名的分支"));
    app.submit_form_dialog();
    assert!(app.form_dialog.is_some());
    app.edit_field(|f| {
        f.set_text("side");
        true
    });
    app.form_dialog.as_mut().unwrap().checkout = false;
    app.submit_form_dialog();
    assert_eq!(head(&app), "featurex");
    assert!(app.git.branches.iter().any(|b| b.name == "side"));

    // 菜单里：当前分支只能「从这里新建」，别的分支能检出、新建、比较、合并、变基、删除。
    let menu = branch_menu(&app.git.branches, &app.git.stashes, &[], &[], None, false);
    let sub = |title: &str| {
        menu.iter()
            .find_map(|e| match e {
                MenuEntry::Submenu(t, items) | MenuEntry::CheckedSubmenu(t, items)
                    if t == title =>
                {
                    Some(items.clone())
                }
                _ => None,
            })
            .unwrap()
    };
    // 当前分支打勾（勾在系统菜单的勾选列里，不写进名字）。
    assert!(
        menu.iter()
            .any(|e| matches!(e, MenuEntry::CheckedSubmenu(t, _) if t == "featurex"))
    );
    // 没有上游：拉取灰显。
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::GitPull, false)));
    assert_eq!(sub("featurex").len(), 1);
    assert_eq!(sub("main").len(), 7);

    // 检出 main，再删除 featurex。
    let main = index_of(&app, "main");
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Checkout, main));
    assert_eq!(head(&app), "main");
    let feature = index_of(&app, "featurex");
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Delete, feature));
    assert!(app.git.branches.iter().all(|b| b.name != "featurex"));

    // 菜单打开期间分支列表刷新、下标错位：按弹出菜单时的列表执行，删除的仍是菜单上的分支。
    git_in(&root, &["branch", "aaa"]);
    git_in(&root, &["branch", "doomed"]);
    app.git_refresh();
    let doomed = index_of(&app, "doomed");
    app.open_context_menu(
        crate::app::input::context_menu::MenuTarget::Branch,
        (0.0, 0.0),
    );
    git_in(&root, &["branch", "--delete", "aaa"]);
    app.git_refresh();
    assert_ne!(index_of(&app, "doomed"), doomed);
    app.run_menu_command(MenuCommand::GitBranch(BranchOp::Delete, doomed));
    assert!(app.git.branches.iter().all(|b| b.name != "doomed"));
    assert!(app.git.branches.iter().any(|b| b.name == "side"));

    // 没有远程：推送说明原因。
    app.run_branch_command(MenuCommand::GitPush);
    assert!(
        app.notice
            .as_deref()
            .is_some_and(|n| n.contains("未配置远程仓库")),
        "{:?}",
        app.notice
    );

    // 加一个远程：第一次推送设好上游，之后获取、拉取都能用，远程分支出现在列表里。
    let remote = root.with_extension("remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    git_in(
        root.parent().unwrap(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git_in(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    app.run_branch_command(MenuCommand::GitPush);
    app.git_refresh();
    assert!(
        app.git
            .branches
            .iter()
            .any(|b| b.current && b.upstream.is_some())
    );
    app.run_branch_command(MenuCommand::GitFetch);
    app.run_branch_command(MenuCommand::GitPull);
    assert!(
        app.git
            .branches
            .iter()
            .any(|b| b.remote && b.name == "origin/main")
    );

    // 远程分支检出成跟踪该远程分支的本地分支。
    git_in(&root, &["push", "-q", "origin", "main:topic"]);
    app.run_branch_command(MenuCommand::GitFetch);
    let topic = index_of(&app, "origin/topic");
    choose_in_branch_menu(&mut app, MenuCommand::GitBranch(BranchOp::Checkout, topic));
    assert_eq!(head(&app), "topic");
    paint(&mut app);

    // 提交并推送：提交成功以后继续推送，本地不再领先远程。
    std::fs::write(root.join("src/main.rs"), "fn main() { pushed() }\n").unwrap();
    app.git_refresh();
    app.run_action(Action::ShowCommit);
    app.edit_field(|f| f.insert("提交并推送"));
    app.run_commit_button_command(crate::app::input::context_menu::MenuCommand::CommitPush);
    app.git_refresh();
    let status = app.git.status.as_ref().unwrap();
    assert!(status.entries.is_empty(), "{:?}", status.entries);
    assert_eq!(status.ahead, 0);
    paint(&mut app);

    let _ = std::fs::remove_dir_all(&remote);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：新建分支对话框的空、已填写、重名（错误色）、基于日志里的提交（「基于」不可选）四种状态，
/// 浅色与深色各一张。
#[test]
#[ignore = "截图，手动运行"]
fn snapshot_branch_dialog() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("branch-dialog-{theme}"), false, dark)
        else {
            return;
        };
        app.open_branch_dialog(None, None);
        shoot(&mut app, &format!("branch-dialog-empty-{theme}"));
        app.edit_field(|f| f.insert("feat/login-page"));
        shoot(&mut app, &format!("branch-dialog-{theme}"));
        app.edit_field(|f| {
            f.set_text("main");
            true
        });
        shoot(&mut app, &format!("branch-dialog-error-{theme}"));
        app.close_form_dialog();
        let oid = app.git.status.as_ref().unwrap().oid.clone().unwrap();
        app.open_branch_dialog(None, Some(oid));
        app.edit_field(|f| f.insert("hotfix"));
        shoot(&mut app, &format!("branch-dialog-commit-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn a_running_branch_operation_slides_into_the_status_bar() {
    let Some((mut app, root)) = git_app("git-status-running", false) else {
        return;
    };
    app.status_motion.animate_without_window = true;
    paint(&mut app);
    // 打开时各项出现的过渡直接走完。
    let slide = crate::app::status::motion::STATUS_SLIDE;
    app.status_motion.freeze(slide);
    app.status_motion.settle();
    paint(&mut app);
    assert!(!app.status_motion.moving());
    app.branch.running = Some(winc_i18n::t!("git-op-sync").to_owned());
    paint(&mut app);
    assert!(app.status_motion.moving());
    let _ = std::fs::remove_dir_all(&root);
}
