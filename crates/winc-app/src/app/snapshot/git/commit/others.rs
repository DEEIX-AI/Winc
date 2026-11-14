//! 「显示全部仓库」的测试：其他仓库的更改列在当前仓库之后，点击即切换过去。

use super::*;
use crate::app::OtherRow;
use crate::app::input::context_menu::{MenuCommand, MenuEntry};

/// 建一个放着两个仓库 `a`、`b` 的目录并打开，各有一个改过的文件和一个未跟踪的文件。
fn two_repositories(name: &str, dark: bool) -> Option<(App, PathBuf, PathBuf)> {
    let dir = search_folder(name);
    for repo in ["a", "b"] {
        let repo = dir.join(repo);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() { }\n").unwrap();
        if !make_repo(&repo) {
            return None;
        }
    }
    let mut app = app_with(&dir, dark);
    app.config.repository_scan_depth = Some(1);
    app.settings_toggle(Setting::GitModule);
    if git_unusable(&app) {
        return None;
    }
    let dir = app.editor.root().unwrap().to_path_buf();
    Some((app, dir.join("a"), dir.join("b")))
}

#[test]
fn other_repositories_are_listed_after_the_current_one_and_a_click_switches() {
    let Some((mut app, a, b)) = two_repositories("git-all-repositories", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git.root.as_ref(), Some(&a));

    // 没有选过时随样式：暂存区视图只显示当前仓库，勾选式列出其他仓库。有多个仓库时「⋯」菜单
    // 里有开关，关掉以后只显示当前仓库。
    let toggle = MenuCommand::CommitAllRepositories;
    assert!(
        app.commit_more_menu()
            .contains(&MenuEntry::Item(toggle, true))
    );
    app.git.commit.set_staging(true);
    assert!(!app.menu_checked(toggle) && app.commit_other_rows().is_empty());
    app.git.commit.set_staging(false);
    assert!(app.menu_checked(toggle) && !app.commit_other_rows().is_empty());
    app.run_commit_more_command(toggle);
    assert!(!app.menu_checked(toggle) && app.commit_other_rows().is_empty());
    let alone = app.commit_max_scroll();
    app.run_commit_more_command(toggle);
    assert!(app.menu_checked(toggle));

    // 另一个仓库的标题行和它的两个文件排在当前仓库的更改之后，一起滚动。
    let listed = [
        OtherRow::Repository(1),
        OtherRow::File(1, 0),
        OtherRow::File(1, 1),
    ];
    assert_eq!(app.commit_other_rows(), listed);
    app.layout.sidebar.h = 200.0;
    assert!(app.commit_max_scroll() > alone);
    app.relayout();
    paint(&mut app);
    let rows = app.commit_rows().len();
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let header = hit(&app, CommitTarget::OtherRepository(1));
    let last = hit(&app, CommitTarget::File(1));
    assert_eq!(header.y, last.y + COMMIT_ROW);
    assert_eq!(rows, 4);

    // 点另一个仓库的文件：切换到那个仓库，选中该文件并预览差异；原来的仓库列到后面。
    let file = hit(&app, CommitTarget::OtherFile(1, 0));
    click(&mut app, (file.x + 60.0, file.y + 4.0));
    assert_eq!(app.git.root.as_ref(), Some(&b));
    let main = b.join("src/main.rs");
    assert_eq!(app.git.commit.selected.as_deref(), Some(main.as_path()));
    assert_eq!(
        app.editor.active().unwrap().title.as_deref(),
        Some("main.rs · 差异")
    );
    assert_eq!(app.commit_other_rows()[0], OtherRow::Repository(0));

    // 当前仓库没有更改时不画空状态：先写一行说明，再列出其他仓库。
    git_in(&b, &["checkout", "-q", "--", "."]);
    std::fs::remove_file(b.join("src/new.rs")).unwrap();
    app.git_refresh();
    assert_eq!(
        app.commit_other_rows()[..2],
        [OtherRow::Clean, OtherRow::Repository(0)]
    );
    paint(&mut app);
    let header = hit(&app, CommitTarget::OtherRepository(0));
    click(&mut app, (header.x + 60.0, header.y + 4.0));
    assert_eq!(app.git.root.as_ref(), Some(&a));

    // 关闭后只剩当前仓库。
    app.run_commit_more_command(toggle);
    assert!(app.commit_other_rows().is_empty());
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 勾选式下其他仓库的文件有勾选框，默认不勾；勾上的文件随当前仓库的一起提交，各自提交到所在
/// 的仓库，用同一条提交信息。点勾选框不切换仓库。
#[test]
fn checked_files_of_other_repositories_are_committed_alongside() {
    let Some((mut app, a, b)) = two_repositories("git-commit-other-repositories", false) else {
        return;
    };
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.other_group_state(1), None);
    assert_eq!(app.other_checked_count(), 0);
    let subject = |repo: &Path| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["log", "-1", "--format=%s"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let before = subject(&b);

    // 勾上另一个仓库改过的文件：仓库的勾选框成为部分勾选，当前仓库不变，按钮的文件数加一。
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let own = app.commit_scope();
    let check = hit(&app, CommitTarget::OtherCheck(1, 0));
    click(&mut app, (check.x + 8.0, check.y + 8.0));
    assert_eq!(app.git.root.as_ref(), Some(&a));
    assert_eq!(app.other_group_state(1), Some(false));
    assert_eq!(app.other_checked_count(), 1);
    assert_ne!(app.commit_scope(), own);

    // 仓库标题行的勾选框：全部勾上，再点一次全部取消。
    paint(&mut app);
    let group = hit(&app, CommitTarget::OtherGroupCheck(1));
    click(&mut app, (group.x + 8.0, group.y + 8.0));
    assert_eq!(app.other_group_state(1), Some(true));
    click(&mut app, (group.x + 8.0, group.y + 8.0));
    assert_eq!(app.other_group_state(1), None);
    app.press_commit(CommitTarget::OtherCheck(1, 0));

    app.git.commit.message.set_text("两个仓库一起提交");
    app.git_commit();
    assert_eq!(subject(&a), "两个仓库一起提交");
    assert_eq!(subject(&b), "两个仓库一起提交");
    assert_ne!(before, subject(&b));
    assert_eq!(app.other_checked_count(), 0);

    // 没勾的文件留在另一个仓库里，没有被提交。
    assert!(b.join("src/new.rs").exists());
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&b)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "?? src/new.rs");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 其他仓库标题行上的分支可以直接点：切换到那个仓库，并展开它的分支选择面板。
#[test]
fn the_branch_on_another_repository_opens_its_branch_picker() {
    let Some((mut app, a, b)) = two_repositories("git-other-branch", false) else {
        return;
    };
    git_in(&b, &["branch", "-q", "topic"]);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    assert_eq!(app.git.root.as_ref(), Some(&a));
    let hit = |app: &App, t: CommitTarget| app.hits.commit.iter().find(|(_, h)| *h == t).unwrap().0;
    let chip = hit(&app, CommitTarget::OtherBranch(1));
    let header = hit(&app, CommitTarget::OtherRepository(1));
    assert!(chip.x > header.x && chip.right() < header.right());
    click(&mut app, (chip.x + 4.0, chip.y + 4.0));
    assert_eq!(app.git.root.as_ref(), Some(&b));
    assert!(app.picker_lists_branches());
    assert!(app.git.branches.iter().any(|branch| branch.name == "topic"));
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 截图：开着「显示全部仓库」的提交视图（`commit-all-repositories-*`）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_all_repositories() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let name = format!("commit-all-repositories-{theme}");
        let Some((mut app, a, _)) = two_repositories(&name, dark) else {
            return;
        };
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        app.press_commit(CommitTarget::OtherCheck(1, 0));
        shoot(&mut app, &name);
        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }
}
