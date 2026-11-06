//! `wsl` 的测试：识别发行版中的路径，以及两种路径写法之间的换算。

use super::*;

fn place(text: &str) -> Option<(String, String)> {
    locate(Path::new(text)).map(|p| (p.distro, p.path))
}

fn found(distro: &str, path: &str) -> Option<(String, String)> {
    Some((distro.to_owned(), path.to_owned()))
}

#[test]
fn both_host_names_are_recognized() {
    let want = found("Ubuntu", "/home/me/app");
    assert_eq!(place(r"\\wsl.localhost\Ubuntu\home\me\app"), want);
    assert_eq!(place(r"\\wsl$\Ubuntu\home\me\app"), want);
    assert_eq!(place(r"\\WSL.LOCALHOST\Ubuntu\home\me\app"), want);
    assert_eq!(place("//wsl.localhost/Ubuntu/home/me/app"), want);
    assert_eq!(place(r"\\?\UNC\wsl.localhost\Ubuntu\home\me\app"), want);
}

#[test]
fn the_root_of_a_distribution_is_slash() {
    assert_eq!(place(r"\\wsl.localhost\Ubuntu"), found("Ubuntu", "/"));
    assert_eq!(place(r"\\wsl.localhost\Ubuntu\"), found("Ubuntu", "/"));
    assert_eq!(place(r"\\wsl.localhost"), None);
}

#[test]
fn other_paths_are_not_in_a_distribution() {
    for text in [
        r"C:\Users\me",
        r"\\server\share\dir",
        "/home/me",
        "relative/wsl.localhost",
        "",
    ] {
        assert_eq!(place(text), None, "{text}");
    }
}

#[test]
fn paths_convert_back_with_the_same_host() {
    for root in [r"\\wsl.localhost\Debian\work", r"\\wsl$\Debian\work"] {
        let location = locate(Path::new(root)).unwrap();
        let back = location.windows_path(&location.path);
        assert_eq!(back, PathBuf::from(root));
        assert_eq!(locate(&back), Some(location));
    }
}

#[test]
fn names_with_spaces_stay_whole() {
    let path = r"\\wsl.localhost\Ubuntu-22.04\home\my files";
    assert_eq!(place(path), found("Ubuntu-22.04", "/home/my files"));
}

/// 其他平台不把这种写法视为发行版中的路径，`git` 的路径按普通路径处理。
#[cfg(not(windows))]
#[test]
fn only_windows_uses_the_conversion() {
    let root = Path::new(r"\\wsl.localhost\Ubuntu\repo");
    assert_eq!(locate_here(root), None);
    assert_eq!(resolve(Path::new("/repo"), ".git"), Path::new("/repo/.git"));
    assert_eq!(git_arg(Path::new("/repo"), Path::new("/repo/a")), "/repo/a");
    assert!(!available());
    assert_eq!(temp_dir(Path::new("/repo")), std::env::temp_dir());
    assert_eq!(temp_dir(root), std::env::temp_dir());
}

#[test]
fn the_temporary_index_reaches_git_in_the_distribution() {
    assert!(GIT_ENV.split(':').any(|name| name == "GIT_INDEX_FILE"));
}

#[cfg(windows)]
#[test]
fn windows_converts_paths_inside_a_distribution() {
    let root = Path::new(r"\\wsl.localhost\Ubuntu\repo");
    assert_eq!(
        resolve(root, "/home/me/repo/.git"),
        PathBuf::from(r"\\wsl.localhost\Ubuntu\home\me\repo\.git")
    );
    assert_eq!(resolve(root, ".git"), root.join(".git"));
    let other = Path::new(r"\\wsl.localhost\Ubuntu\repo.worktrees\feat");
    assert_eq!(git_arg(root, other), "/repo.worktrees/feat");
    let temp = temp_dir(root);
    assert_eq!(temp, PathBuf::from(r"\\wsl.localhost\Ubuntu\tmp"));
    assert_eq!(
        locate_here(&temp).map(|place| place.path).as_deref(),
        Some("/tmp")
    );
}
