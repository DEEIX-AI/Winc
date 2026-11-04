//! 分支菜单的测试：当前分支打勾、远程分支分组、被占用的分支不能签出到新工作树。

use super::*;

impl App {
    /// 测试用：在 `root` 里做完了「拉取」，收下结果。
    #[cfg(unix)] // 只有 Unix 上的 Git 测试用到
    pub(in crate::app) fn pulled_in(&mut self, root: &Path) {
        let what = "拉取".to_owned();
        // 换目录之前起的操作：编号比后来起的都小。
        let _ = self.branch.results.0.send(Done {
            id: 0,
            root: root.into(),
            what,
            result: Ok((String::new(), Then::Nothing)),
        });
        self.poll_branch_ops();
    }
}

fn branch(name: &str, remote: bool, current: bool, upstream: Option<&str>) -> Branch {
    Branch {
        name: name.to_owned(),
        remote,
        current,
        upstream: upstream.map(str::to_owned),
        oid: String::new(),
    }
}

#[test]
fn the_current_branch_is_ticked_and_remote_branches_sit_under_their_remote() {
    let branches = [
        branch("main", false, true, Some("origin/main")),
        branch("dev", false, false, None),
        branch("origin/main", true, false, None),
        branch("origin/dev", true, false, None),
        branch("fork/topic", true, false, None),
    ];
    let menu = branch_menu(&branches, &[], &[], &[], None, false);
    // 当前分支打勾，名字里没有「✓」；有上游时能拉取。
    assert!(
        menu.iter()
            .any(|e| matches!(e, MenuEntry::CheckedSubmenu(t, _) if t == "main"))
    );
    assert!(menu.contains(&MenuEntry::Item(MenuCommand::GitPull, true)));
    // 远程分支按远程收进子菜单，里面去掉远程名。
    let remote = |name: &str| {
        menu.iter()
            .find_map(|e| match e {
                MenuEntry::Submenu(t, items) if t == name => Some(items.clone()),
                _ => None,
            })
            .unwrap()
    };
    let titles = |items: Vec<MenuEntry>| -> Vec<String> {
        items
            .into_iter()
            .filter_map(|e| match e {
                MenuEntry::Submenu(t, _) => Some(t),
                _ => None,
            })
            .collect()
    };
    assert_eq!(titles(remote("origin")), ["main", "dev"]);
    assert_eq!(titles(remote("fork")), ["topic"]);
}

#[test]
fn branches_held_by_another_worktree_cannot_go_into_a_new_one() {
    use MenuCommand::GitBranch;
    let branches = [
        branch("main", false, true, None),
        branch("dev", false, false, None),
        branch("feat", false, false, None),
        branch("origin/feat", true, false, None),
        branch("origin/main", true, false, None),
        branch("gone", false, false, None),
    ];
    fn flatten(entries: &[MenuEntry], out: &mut Vec<MenuEntry>) {
        for entry in entries {
            match entry {
                MenuEntry::Submenu(_, items) | MenuEntry::CheckedSubmenu(_, items) => {
                    flatten(items, out)
                }
                other => out.push(other.clone()),
            }
        }
    }
    let mut items = Vec::new();
    flatten(
        &branch_menu(&branches, &[], &["feat"], &["gone"], None, false),
        &mut items,
    );
    // 当前分支、被占用的（远程分支看同名的本地分支，`origin/main` 在 `main` 上）没有「在新
    // 工作树中签出」；顶上有「新建工作树…」。
    assert!(items.contains(&MenuEntry::Item(MenuCommand::NewWorktree, true)));
    // 被目录已不在的工作树占着：签出、删除都灰掉（git 不让），要先清理。
    for op in [BranchOp::Checkout, BranchOp::Delete] {
        assert!(items.contains(&MenuEntry::Item(GitBranch(op, 5), false)));
    }
    let offered: Vec<usize> = items
        .iter()
        .filter_map(|e| match e {
            MenuEntry::Item(GitBranch(BranchOp::NewWorktree, i), true) => Some(*i),
            _ => None,
        })
        .collect();
    assert_eq!(offered, [1]);
}
