//! 在 `PATH` 中查找程序的测试：只考虑绝对路径的目录、Windows 上按 `PATHEXT` 补扩展名。

use super::*;
#[cfg(unix)]
use crate::test_util::scratch;
#[cfg(unix)]
use crate::tools::format::tests::fake_tool;

#[cfg(unix)] // 用不带扩展名的脚本冒充工具；Windows 上程序要带 PATHEXT 中的扩展名
#[test]
fn programs_are_found_in_absolute_path_directories() {
    let root = scratch("find-in-path");
    let tool = fake_tool(&root.join("bin"), "prettier", "cat");
    let path = std::env::join_paths([root.join("nope"), root.join("bin")]).unwrap();
    assert_eq!(find_in_path("prettier", Some(&path)), Some(tool.clone()));
    assert!(find_in_path("jq", Some(&path)).is_none());
    assert!(find_in_path("prettier", None).is_none());
    // 相对路径的目录不算：其含义随当前目录而变。
    assert!(find_in_path("prettier", Some(OsStr::new("bin"))).is_none());

    // `Shell::find` 在给定环境的 `PATH` 里找，不运行登录 shell。
    let shell = Shell::with_vars(vec![("PATH".into(), path)]);
    assert!(shell.is_ready());
    assert_eq!(shell.find("prettier"), Some(tool));
    assert_eq!(shell.find("jq"), None);
    assert_eq!(Shell::with_vars(Vec::new()).find("prettier"), None);
}

#[test]
fn windows_programs_are_looked_up_with_their_extensions() {
    assert_eq!(
        windows_names("prettier", ".COM;.EXE;.BAT;.CMD;.VBS"),
        [
            "prettier.com",
            "prettier.exe",
            "prettier.bat",
            "prettier.cmd",
            "prettier.vbs"
        ]
    );
    assert_eq!(windows_names("x", ";;EXE;.Cmd"), ["x.cmd"]);
    // 其他系统上就是程序名本身。
    if cfg!(not(windows)) {
        assert_eq!(file_names("prettier", None), ["prettier"]);
    }
}
