//! 工作树列表和菜单的测试：主工作树、被占用的分支、菜单项的可用状态。

use std::path::PathBuf;

use super::*;

fn worktree(path: &str, head: Option<Head>) -> Worktree {
    Worktree {
        path: PathBuf::from(path),
        head,
        locked: false,
        prunable: false,
    }
}

fn branch(name: &str) -> Option<Head> {
    Some(Head::Branch(name.to_owned()))
}

#[test]
fn the_current_worktree_main_and_held_branches() {
    // 打开的是主工作树：不算附属的，别的工作树签出的分支算被占用。
    let mut git = GitState {
        root: Some(PathBuf::from("/r/winc")),
        worktrees: vec![
            worktree("/r/winc", branch("main")),
            worktree("/r/winc.worktrees/feat", branch("feat")),
            worktree("/r/fix", Some(Head::Detached("abc1234".into()))),
        ],
        ..GitState::default()
    };
    assert_eq!(git.current_worktree(), Some(0));
    assert!(git.main_worktree().is_none());
    assert_eq!(git.held_branches(), ["feat"]);
    assert_eq!(git.worktree_holding("feat"), Some(1));
    assert_eq!(git.worktree_holding("main"), None);

    // 打开的是附属工作树：主工作树是第一项，自己签出的分支不算被占用。
    git.root = Some(PathBuf::from("/r/winc.worktrees/feat"));
    assert_eq!(git.current_worktree(), Some(1));
    assert_eq!(
        git.main_worktree().map(worktree_name).as_deref(),
        Some("winc")
    );
    assert_eq!(git.held_branches(), ["main"]);
    assert_eq!(git.worktree_holding("feat"), None);

    // 只有一个工作树：列表为空，什么都不算。
    git.worktrees.clear();
    assert_eq!(git.current_worktree(), None);
    assert!(git.held_branches().is_empty());
}

#[test]
fn worktree_entries_list_openable_worktrees_with_alternates() {
    use MenuCommand::*;
    // 不在仓库里：什么都没有；只有一个工作树：只有「新建工作树…」。
    let single = [worktree("/r/winc", branch("main"))];
    assert!(worktree_entries(&single, Some(0), false, false).is_empty());
    assert_eq!(
        worktree_entries(&[], None, true, false),
        [MenuEntry::Item(NewWorktree, true), MenuEntry::Separator]
    );

    let mut gone = worktree("/r/gone", branch("old"));
    gone.prunable = true;
    let mut locked = worktree("/r/c", branch("c"));
    locked.locked = true;
    let list = [
        worktree("/r/winc.git", None),
        worktree("/r/a", branch("a")),
        worktree("/r/b", branch("b")),
        gone,
        locked,
    ];
    // 裸仓库的主工作树不列；当前这个没有「在当前窗口打开」；目录不在的灰掉、没有备选项。
    // 移除：主工作树、目录不在的不列，当前的、锁定的灰掉；有目录不在的时能清理。
    assert_eq!(
        worktree_entries(&list, Some(1), true, false),
        [
            MenuEntry::Item(WorktreeHeading, false),
            MenuEntry::Item(OpenWorktree(1), true),
            MenuEntry::Item(OpenWorktree(2), true),
            MenuEntry::Alternate(OpenWorktreeHere(2), true),
            MenuEntry::Item(OpenWorktree(3), false),
            MenuEntry::Item(OpenWorktree(4), true),
            MenuEntry::Alternate(OpenWorktreeHere(4), true),
            MenuEntry::Item(NewWorktree, true),
            MenuEntry::Submenu(
                "移除工作树".to_owned(),
                vec![
                    MenuEntry::Item(RemoveWorktree(1), false),
                    MenuEntry::Item(RemoveWorktree(2), true),
                    MenuEntry::Item(RemoveWorktree(4), false),
                ]
            ),
            MenuEntry::Item(PruneWorktrees, true),
            MenuEntry::Separator,
        ]
    );
    // 有 Git 操作在运行：会改动仓库的都灰掉，打开照常。
    let busy = worktree_entries(&list, Some(1), true, true);
    assert!(busy.contains(&MenuEntry::Item(NewWorktree, false)));
    assert!(busy.contains(&MenuEntry::Item(PruneWorktrees, false)));
    assert!(busy.contains(&MenuEntry::Item(OpenWorktree(2), true)));
    assert_eq!(worktree_name(&list[0]), "winc");
}
