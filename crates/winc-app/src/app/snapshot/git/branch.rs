//! 分支菜单与新建分支对话框的测试：新建、切换、推送、删除，以及远程分支与提交并推送。

use super::*;

#[test]
fn branch_menu_creates_switches_pushes_and_deletes() {
    use crate::app::git::branch::{BranchOp, branch_menu, branch_ops};
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
    // 没有上游：拉取灰显。
    let menu = branch_menu(&app.git.branches, &app.git.stashes, None, false);
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::GitPull, false)));
    let ops = |app: &App, name: &str| {
        branch_ops(index_of(app, name), &app.git.branches, &[], &[], false).len()
    };
    assert_eq!(ops(&app, "featurex"), 1);
    assert_eq!(ops(&app, "main"), 7);

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
        crate::app::input::context_menu::MenuTarget::BranchOps(doomed),
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

/// 分支选择器测试用的仓库：`feature/login`、`feature/search`、`fix/one` 三个分支，当前在 `main`。
fn picker_repo(name: &str, dark: bool) -> Option<(App, PathBuf)> {
    let (mut app, root) = git_app_in(name, false, dark)?;
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "all"]);
    for branch in ["feature/login", "feature/search", "fix/one"] {
        git_in(&root, &["branch", branch]);
    }
    app.git_refresh();
    Some((app, root))
}

fn picker_labels(app: &App) -> Vec<String> {
    use crate::app::git::branch::picker::BranchRow;
    app.picker_branch_rows
        .as_deref()
        .unwrap()
        .iter()
        .filter_map(|row| match row {
            BranchRow::Folder { label, .. } | BranchRow::Branch { label, .. } => {
                Some(label.clone())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn clicking_the_branch_name_opens_the_picker_and_enter_checks_out() {
    let Some((mut app, root)) = picker_repo("pk", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    // 项目名很长时侧栏不画分支名，这里用短的项目名。
    let hit = app.branch_hit;
    assert!(hit.w > 0.0);
    click(&mut app, (hit.x + 4.0, hit.y + 4.0));
    assert!(app.picker_lists_branches());
    assert_eq!(app.focus, Focus::Picker);
    assert_eq!(picker_labels(&app)[..3], ["feature", "login", "search"]);
    paint(&mut app);
    // 分支行数多于文件列表的上限时，面板也放得下。
    assert!(app.layout.picker.h > 0.0);

    // 上下键跳过分组标题；回车签出分支并关闭选择器。
    app.picker_move(false);
    let selected = app.picker_selected;
    assert!(
        app.picker_branch_rows.as_deref().unwrap()[selected]
            != crate::app::git::branch::picker::BranchRow::Heading(
                crate::app::git::branch::picker::BranchSection::Local
            )
    );
    app.picker_input.set_text("search");
    app.picker_refresh();
    assert_eq!(picker_labels(&app), ["feature", "search"]);
    app.picker_accept();
    assert!(!app.picker_open);
    let current = app.git.branches.iter().find(|b| b.current);
    assert_eq!(current.map(|b| b.name.as_str()), Some("feature/search"));

    // 切换过的分支出现在最近使用里，当前分支不列出。
    app.open_branch_picker();
    use crate::app::git::branch::picker::{BranchRow, BranchSection};
    let rows = app.picker_branch_rows.clone().unwrap();
    assert_eq!(rows[1], BranchRow::Heading(BranchSection::Recent));
    assert!(matches!(&rows[2], BranchRow::Branch { label, .. } if label == "main"));

    // 再按分支名关闭（点在面板之外）。
    paint(&mut app);
    click(&mut app, (hit.x + 4.0, hit.y + 4.0));
    assert!(!app.picker_open);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_branch_picker_folds_folders_and_opens_branch_actions() {
    use crate::app::git::branch::BranchOp;
    use crate::app::git::branch::picker::more_button;
    use crate::app::input::context_menu::MenuCommand;

    let Some((mut app, root)) = picker_repo("git-picker-fold", false) else {
        return;
    };
    app.open_branch_picker();
    paint(&mut app);
    // 第一个分支行在 `feature` 下面；左键回到 `feature`，再按收起，右键展开。
    assert_eq!(picker_labels(&app)[..2], ["feature", "login"]);
    assert!(app.branch_picker_side(false));
    assert!(app.branch_picker_side(false));
    assert!(!picker_labels(&app).contains(&"login".to_owned()));
    assert!(app.branch_picker_side(true));
    assert!(picker_labels(&app).contains(&"login".to_owned()));
    // 输入查询时左右键交给输入框。
    app.picker_input.set_text("f");
    app.picker_refresh();
    assert!(!app.branch_picker_side(true));
    app.picker_input.set_text("");
    app.picker_refresh();

    // 点「⋯」打开分支的操作菜单，选择器还在；选了一项才关闭。
    let index = app.picker_selected;
    let parts = crate::view::picker_parts(&app, app.layout.picker);
    let button = more_button(parts.row(index, app.picker_scroll));
    app.click_branch_row(index, button.x + 2.0);
    assert!(app.picker_lists_branches());
    let branches = &app.git.branches;
    let login = branches
        .iter()
        .position(|b| b.name == "feature/login")
        .unwrap();
    let delete = MenuCommand::GitBranch(BranchOp::Delete, login);
    assert!(menu_commands(&app).contains(&(delete, true)));
    app.run_menu_command(delete);
    assert!(!app.picker_open);
    assert!(app.git.branches.iter().all(|b| b.name != "feature/login"));
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：分支选择器，深浅两套配色。
#[test]
#[ignore]
fn shoot_branch_picker() {
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        let Some((mut app, root)) = picker_repo(&format!("git-picker-shot-{theme}"), dark) else {
            return;
        };
        for branch in ["fix/deep/two", "release/1.0", "release/2.0"] {
            git_in(&root, &["branch", branch]);
        }
        git_in(&root, &["checkout", "-q", "fix/one"]);
        git_in(&root, &["checkout", "-q", "main"]);
        git_in(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        app.git_refresh();
        app.open_branch_picker();
        shoot(&mut app, &format!("branch-picker-{theme}"));
        app.picker_input.set_text("rel");
        app.picker_refresh();
        shoot(&mut app, &format!("branch-picker-search-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
