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

    // 默认只显示当前仓库；有多个仓库时「⋯」菜单里有开关。
    assert!(app.commit_other_rows().is_empty());
    let toggle = MenuCommand::CommitAllRepositories;
    assert!(
        app.commit_more_menu()
            .contains(&MenuEntry::Item(toggle, true))
    );
    assert!(!app.menu_checked(toggle));
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
        app.run_commit_more_command(MenuCommand::CommitAllRepositories);
        shoot(&mut app, &name);
        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }
}
