//! `picker` 的测试：按 `/` 分层、最近使用、搜索时展开、远程默认收起。

use super::*;

fn branch(name: &str, remote: bool, current: bool) -> Branch {
    Branch {
        name: name.to_owned(),
        remote,
        current,
        upstream: None,
        oid: String::new(),
    }
}

fn sample() -> Vec<Branch> {
    vec![
        branch("feature/login", false, true),
        branch("feature/search", false, false),
        branch("fix/deep/one", false, false),
        branch("main", false, false),
        branch("origin/main", true, false),
        branch("origin/feature/login", true, false),
    ]
}

/// 把各行写成缩进的文字，便于比较。
fn outline(rows: &[BranchRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            BranchRow::Actions => "actions".to_owned(),
            BranchRow::Heading(section) => format!("{section:?}"),
            BranchRow::Folder {
                label,
                depth,
                count,
                open,
                ..
            } => {
                let mark = if *open { "v" } else { ">" };
                format!("{}{mark} {label} {count}", "  ".repeat(*depth))
            }
            BranchRow::Branch { depth, label, .. } => format!("{}{label}", "  ".repeat(*depth)),
        })
        .collect()
}

#[test]
fn branches_form_a_tree_by_slash_and_remotes_start_collapsed() {
    let rows = branch_rows(&sample(), &[], &[], &HashSet::new());
    assert_eq!(
        outline(&rows),
        [
            "actions",
            "Local",
            "v feature 2",
            "  login",
            "  search",
            "v fix/deep 1",
            "  one",
            "main",
            "Remote",
            "> origin 2",
        ]
    );
}

#[test]
fn toggled_folders_flip_their_default_state() {
    let toggled = HashSet::from([(false, "feature".to_owned()), (true, "origin".to_owned())]);
    let rows = branch_rows(&sample(), &[], &[], &toggled);
    assert_eq!(
        outline(&rows),
        [
            "actions",
            "Local",
            "> feature 2",
            "v fix/deep 1",
            "  one",
            "main",
            "Remote",
            "v origin 2",
            "  v feature 1",
            "    login",
            "  main",
        ]
    );
}

#[test]
fn recent_lists_existing_local_branches_without_the_current_one() {
    let recent: Vec<String> = [
        "feature/login",
        "gone",
        "main",
        "origin/main",
        "feature/search",
    ]
    .map(str::to_owned)
    .into();
    let rows = branch_rows(&sample(), &recent, &[], &HashSet::new());
    assert_eq!(
        outline(&rows)[..4],
        ["actions", "Recent", "main", "feature/search"]
    );
}

#[test]
fn recent_keeps_at_most_five_branches() {
    let branches: Vec<Branch> = (0..8)
        .map(|i| branch(&format!("b{i}"), false, false))
        .collect();
    let recent: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
    let rows = branch_rows(&branches, &recent, &[], &HashSet::new());
    let end = rows
        .iter()
        .position(|r| *r == BranchRow::Heading(BranchSection::Local))
        .unwrap();
    assert_eq!(end - 2, RECENT_BRANCHES);
}

#[test]
fn searching_opens_folders_and_hides_everything_else() {
    let toggled = HashSet::from([(false, "feature".to_owned())]);
    let rows = branch_rows(&sample(), &["main".to_owned()], &needle("login"), &toggled);
    assert_eq!(
        outline(&rows),
        [
            "Local",
            "v feature 1",
            "  login",
            "Remote",
            "v origin/feature 1",
            "  login",
        ]
    );
    assert!(branch_rows(&sample(), &[], &needle("zzz"), &HashSet::new()).is_empty());
}

#[test]
fn contiguous_matches_hide_scattered_ones() {
    let branches = [
        branch("feature/login", false, false),
        branch("release/1", false, false),
    ];
    let rows = branch_rows(&branches, &[], &needle("rel"), &HashSet::new());
    assert_eq!(outline(&rows), ["Local", "v release 1", "  1"]);
    // 没有连续命中时仍然列出分散命中的。
    let rows = branch_rows(&branches, &[], &needle("flg"), &HashSet::new());
    assert_eq!(outline(&rows), ["Local", "v feature 1", "  login"]);
}
