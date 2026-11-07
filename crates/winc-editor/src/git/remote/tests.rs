//! `remote` 的测试：目录名的推导、远程列表的解析，以及在真实仓库中的初始化、克隆和远程管理。

use super::*;

#[test]
fn dir_names_come_from_the_last_part_of_the_address() {
    for (url, name) in [
        ("https://github.com/a/repo.git", "repo"),
        ("https://github.com/a/repo", "repo"),
        ("https://github.com/a/repo/", "repo"),
        ("git@github.com:a/repo.git", "repo"),
        ("git@host:repo.git", "repo"),
        ("/srv/git/repo.git", "repo"),
        (r"C:\git\repo.git", "repo"),
    ] {
        assert_eq!(repository_dir_name(url).as_deref(), Some(name), "{url}");
    }
}

#[test]
fn addresses_without_a_usable_name_give_none() {
    for url in [
        "",
        "..",
        "https://host/..",
        "https://host/.git",
        "x/--upload-pack=y",
    ] {
        assert_eq!(repository_dir_name(url), None, "{url}");
    }
}

#[test]
fn remotes_keep_only_fetch_addresses() {
    let text = "origin\thttps://a (fetch)\norigin\thttps://b (push)\nup\tgit@c:d (fetch)\n";
    assert_eq!(
        parse_remotes(text),
        [
            Remote {
                name: "origin".into(),
                url: "https://a".into(),
            },
            Remote {
                name: "up".into(),
                url: "git@c:d".into(),
            },
        ]
    );
}

#[test]
fn a_repository_can_be_created_cloned_and_given_remotes() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("remote");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    git.init(&source).unwrap();
    assert!(source.join(".git").exists());
    git.operate(
        &source,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "first",
        ],
    )
    .unwrap();

    // 克隆后配置了指向来源的 `origin`。
    git.clone_repository(&root, source.to_str().unwrap(), "copy")
        .unwrap();
    let copy = root.join("copy");
    let cancel = Cancel::default();
    let remotes = git.remote_list(&copy, &cancel).unwrap();
    assert_eq!(remotes[0].name, "origin");
    assert_eq!(git.default_remote(&copy).unwrap(), "origin");

    // 目标目录不为空时由 `git` 报错，不覆盖其中的内容。
    assert!(
        git.clone_repository(&root, source.to_str().unwrap(), "copy")
            .is_err()
    );

    // 只有其他名称的远程时使用第一个；没有远程时报错。
    git.operate(&copy, &["remote", "rename", "--", "origin", "up"])
        .unwrap();
    assert_eq!(git.default_remote(&copy).unwrap(), "up");
    git.operate(&copy, &["remote", "remove", "up"]).unwrap();
    assert!(git.default_remote(&copy).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `ext` 协议能运行任意命令，克隆时必须被拒绝。
#[test]
fn cloning_refuses_the_ext_protocol() {
    let shell = Shell::with_vars(std::env::vars_os().collect());
    let Ok(git) = Git::find(&shell) else {
        return;
    };
    let dir = crate::test_util::scratch("remote-ext");
    let root = winc_config::files::canonicalize(&dir).unwrap();
    let marker = root.join("ran");
    let url = format!("ext::sh -c touch% {}", marker.display());
    assert!(git.clone_repository(&root, &url, "copy").is_err());
    assert!(!marker.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
