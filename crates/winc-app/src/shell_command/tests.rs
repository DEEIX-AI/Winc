//! `shell_command` 的测试：链接的位置、安装和卸载的边界情况、授权脚本的转义、前台和后台的判断。

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// 新建测试目录，返回目录和其中的链接位置。
fn sandbox() -> (std::path::PathBuf, Placement) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("winc-shell-{}-{serial}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("Winc.app/Contents/MacOS/winc");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "").unwrap();
    let placement = Placement {
        link: dir.join("bin/winc"),
        target,
    };
    (dir, placement)
}

#[test]
fn only_the_app_bundle_gets_a_link_on_macos() {
    let bundled = Path::new("/Applications/My Apps/winc.app/Contents/MacOS/winc");
    let placement = macos_placement(bundled).unwrap();
    assert_eq!(placement.link, Path::new("/usr/local/bin/winc"));
    assert_eq!(placement.target, bundled);
    // 开发构建不在应用包内。
    assert_eq!(macos_placement(Path::new("/repo/target/debug/winc")), None);
    assert_eq!(macos_placement(Path::new("/x/Contents/MacOS/winc")), None);
}

#[test]
fn the_appimage_links_into_the_home_directory() {
    let placement =
        appimage_placement(Path::new("/opt/winc.AppImage"), Path::new("/home/a")).unwrap();
    assert_eq!(placement.link, Path::new("/home/a/.local/bin/winc"));
    assert_eq!(placement.target, Path::new("/opt/winc.AppImage"));
    assert_eq!(
        appimage_placement(Path::new("winc"), Path::new("/home/a")),
        None
    );
}

#[test]
fn install_creates_the_directory_and_replaces_its_own_link() {
    let (dir, placement) = sandbox();
    assert_eq!(install(&placement), Outcome::Installed);
    assert_eq!(
        std::fs::read_link(&placement.link).unwrap(),
        placement.target
    );
    assert!(!placement.link.with_file_name(".winc.tmp").exists());

    // 应用移动后再次安装：指向旧位置的链接被替换。
    std::fs::remove_file(&placement.link).unwrap();
    let old = dir.join("Old.app/Contents/MacOS/winc");
    std::os::unix::fs::symlink(&old, &placement.link).unwrap();
    assert_eq!(install(&placement), Outcome::Installed);
    assert_eq!(
        std::fs::read_link(&placement.link).unwrap(),
        placement.target
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn another_program_at_the_link_is_left_alone() {
    let (dir, placement) = sandbox();
    std::fs::create_dir_all(placement.link.parent().unwrap()).unwrap();

    // 普通文件。
    std::fs::write(&placement.link, "other").unwrap();
    assert_eq!(install(&placement), Outcome::Occupied);
    assert_eq!(uninstall(&placement), Outcome::Occupied);
    assert_eq!(std::fs::read_to_string(&placement.link).unwrap(), "other");
    std::fs::remove_file(&placement.link).unwrap();

    // 指向其他存在的程序的链接。
    let other = dir.join("other-tool");
    std::fs::write(&other, "").unwrap();
    std::os::unix::fs::symlink(&other, &placement.link).unwrap();
    assert_eq!(install(&placement), Outcome::Occupied);
    assert_eq!(uninstall(&placement), Outcome::Occupied);
    assert_eq!(std::fs::read_link(&placement.link).unwrap(), other);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn uninstall_removes_only_its_own_link() {
    let (dir, placement) = sandbox();
    assert_eq!(uninstall(&placement), Outcome::NotInstalled);
    assert_eq!(install(&placement), Outcome::Installed);
    assert_eq!(uninstall(&placement), Outcome::Uninstalled);
    assert!(std::fs::symlink_metadata(&placement.link).is_err());
    assert!(placement.target.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_read_only_directory_asks_for_authorization() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, placement) = sandbox();
    let bin = placement.link.parent().unwrap();
    std::fs::create_dir_all(bin).unwrap();
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = install(&placement);
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    // root 不受目录权限限制（容器里的测试以 root 运行）。
    assert!(
        matches!(outcome, Outcome::Denied | Outcome::Installed),
        "{outcome:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_authorized_script_quotes_paths() {
    let placement = Placement {
        link: "/usr/local/bin/winc".into(),
        target: "/Applications/Bob's \"Apps\"/winc.app/Contents/MacOS/winc".into(),
    };
    let command = authorized_command(&placement, true);
    assert_eq!(
        command,
        "/bin/mkdir -p '/usr/local/bin' && /bin/ln -sfn \
         '/Applications/Bob'\\''s \"Apps\"/winc.app/Contents/MacOS/winc' '/usr/local/bin/winc'"
    );
    assert_eq!(
        authorized_command(&placement, false),
        "/bin/rm -f '/usr/local/bin/winc'"
    );
    // AppleScript 字符串内的反斜杠和双引号要转义。
    assert_eq!(
        authorized_script(r#"a "b" \c"#),
        r#"do shell script "a \"b\" \\c" with administrator privileges"#
    );
}

#[test]
fn waiting_and_new_instances_stay_in_the_foreground() {
    assert!(!stays_in_foreground(&Args::default()));
    let wait = Args {
        wait: true,
        ..Args::default()
    };
    assert!(stays_in_foreground(&wait));
    let stdin = Args {
        stdin: true,
        ..Args::default()
    };
    assert!(stays_in_foreground(&stdin));
    let new_instance = Args {
        new_instance: true,
        ..Args::default()
    };
    assert!(stays_in_foreground(&new_instance));
}
