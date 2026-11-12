//! 在真实仓库中验证暂存、取消暂存和索引提交，包含初始提交及字面路径。

use super::*;

fn command(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn repository(name: &str) -> Option<(Git, PathBuf)> {
    let git = real_git()?;
    let root = crate::test_util::scratch(name);
    command(&root, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
    ] {
        command(&root, &["config", key, value]);
    }
    Some((git, root))
}

#[test]
fn restore_worktree_preserves_index_before_and_after_first_commit() {
    let Some((git, root)) = repository("git-restore-index") else {
        return;
    };
    let special = root.join("[ab] file.txt");
    let other = root.join("a file.txt");
    let deleted = root.join("deleted.txt");
    let old = root.join("old.txt");
    for path in [&special, &other, &deleted, &old] {
        std::fs::write(path, "first index\n").unwrap();
    }
    command(&root, &["add", "."]);
    let tree = command(&root, &["write-tree"]);
    std::fs::write(&special, "working\n").unwrap();
    std::fs::write(&other, "unrelated\n").unwrap();
    git.restore_worktree(&root, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&special).unwrap(), "working\n");
    git.restore_worktree(&root, std::slice::from_ref(&special))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&special).unwrap(), "first index\n");
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "unrelated\n");
    assert_eq!(command(&root, &["write-tree"]), tree);
    git.commit_staged(&root, "first", false).unwrap();

    std::fs::write(&special, "second index\n").unwrap();
    let added = root.join("added.txt");
    std::fs::write(&added, "added index\n").unwrap();
    git.stage(&root, &[special.clone(), added.clone()]).unwrap();
    command(&root, &["mv", "old.txt", "renamed.txt"]);
    let renamed = root.join("renamed.txt");
    let tree = command(&root, &["write-tree"]);
    for path in [&special, &added, &renamed] {
        std::fs::write(path, "working\n").unwrap();
    }
    std::fs::remove_file(&deleted).unwrap();
    git.restore_worktree(
        &root,
        &[
            special.clone(),
            added.clone(),
            renamed.clone(),
            deleted.clone(),
        ],
    )
    .unwrap();
    assert_eq!(command(&root, &["write-tree"]), tree);
    for (path, expected) in [
        (&special, "second index\n"),
        (&added, "added index\n"),
        (&renamed, "first index\n"),
        (&deleted, "first index\n"),
        (&other, "unrelated\n"),
    ] {
        assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
    }
    assert!(!old.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unstage_unborn_and_partial_staging_preserve_disk_and_other_index_entries() {
    let Some((git, root)) = repository("git-unstage-unborn") else {
        return;
    };
    let special = root.join("[ab] file.txt");
    let other = root.join("a file.txt");
    std::fs::write(&special, "index\n").unwrap();
    std::fs::write(&other, "other\n").unwrap();
    git.stage(&root, &[special.clone(), other.clone()]).unwrap();
    std::fs::write(&special, "working\n").unwrap();
    git.unstage(&root, std::slice::from_ref(&special)).unwrap();
    assert_eq!(std::fs::read_to_string(&special).unwrap(), "working\n");
    assert_eq!(
        command(&root, &["diff", "--cached", "--name-only"]),
        "a file.txt"
    );
    git.stage(&root, std::slice::from_ref(&special)).unwrap();
    git.commit_staged(&root, "first", false).unwrap();

    std::fs::write(&special, "staged\n").unwrap();
    git.stage(&root, std::slice::from_ref(&special)).unwrap();
    std::fs::write(&special, "unstaged\n").unwrap();
    std::fs::write(&other, "unrelated\n").unwrap();
    git.stage(&root, std::slice::from_ref(&other)).unwrap();
    git.unstage(&root, &[]).unwrap();
    git.stage(&root, &[]).unwrap();
    assert_eq!(command(&root, &["show", ":[ab] file.txt"]), "staged");
    git.unstage(&root, std::slice::from_ref(&special)).unwrap();
    assert_eq!(
        command(&root, &["diff", "--cached", "--name-only"]),
        "a file.txt"
    );
    assert_eq!(std::fs::read_to_string(&special).unwrap(), "unstaged\n");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn commit_index_preserves_unstaged_content_and_handles_renames_and_deletions() {
    let Some((git, root)) = repository("git-commit-index") else {
        return;
    };
    for name in ["edit.txt", "old.txt", "delete.txt"] {
        std::fs::write(root.join(name), "base\n").unwrap();
    }
    command(&root, &["add", "."]);
    git.commit_staged(&root, "first", false).unwrap();
    let edited = root.join("edit.txt");
    std::fs::write(&edited, "index\n").unwrap();
    git.stage(&root, std::slice::from_ref(&edited)).unwrap();
    std::fs::write(&edited, "working\n").unwrap();
    command(&root, &["mv", "old.txt", "renamed.txt"]);
    let renamed = [root.join("old.txt"), root.join("renamed.txt")];
    git.unstage(&root, &renamed).unwrap();
    assert!(root.join("renamed.txt").exists());
    assert!(!root.join("old.txt").exists());
    git.stage(&root, &renamed).unwrap();
    std::fs::remove_file(root.join("delete.txt")).unwrap();
    git.stage(&root, &[root.join("delete.txt")]).unwrap();
    git.commit_staged(&root, "second", false).unwrap();
    assert_eq!(command(&root, &["show", "HEAD:edit.txt"]), "index");
    assert_eq!(std::fs::read_to_string(&edited).unwrap(), "working\n");
    assert_eq!(
        command(&root, &["ls-tree", "--name-only", "HEAD"]),
        "edit.txt\nrenamed.txt"
    );
    assert!(command(&root, &["diff", "--cached", "--name-only"]).is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn rejected_index_commit_keeps_staging_and_message() {
    use std::os::unix::fs::PermissionsExt;
    let Some((git, root)) = repository("git-index-hook") else {
        return;
    };
    let path = root.join("a.txt");
    std::fs::write(&path, "index\n").unwrap();
    git.stage(&root, std::slice::from_ref(&path)).unwrap();
    std::fs::write(&path, "working\n").unwrap();
    let hook = root.join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let risks = git.inspect(&root).unwrap();
    git.trust(&root, risks.fingerprint);
    assert!(git.commit_staged(&root, "refused", false).is_err());
    assert_eq!(command(&root, &["show", ":a.txt"]), "index");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "working\n");
    let _ = std::fs::remove_dir_all(root);
}
