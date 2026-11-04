//! 登录 shell 环境的测试：只取两个标记之间的内容、丢弃 shell 自身的状态变量，以及超时与
//! 缺少标记时的失败。

use super::*;
#[cfg(unix)]
use crate::test_util::scratch;
#[cfg(unix)]
use crate::tools::format::tests::{TOOL_TIMEOUT, fake_tool};

#[test]
fn only_what_is_between_the_markers_counts() {
    let output = format!(
        "Welcome!\n{MARKER}PATH=/opt/homebrew/bin:/usr/bin\0LANG=zh_CN.UTF-8\0\
             MULTI=a\nb\0PWD=/tmp\0SHLVL=2\0=broken\0{MARKER}bye\n"
    );
    let vars = parse(output.as_bytes()).unwrap();
    let get = |name: &str| {
        vars.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.to_string_lossy().into_owned())
    };
    assert_eq!(get("PATH").as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
    assert_eq!(get("LANG").as_deref(), Some("zh_CN.UTF-8"));
    // 值中含换行不会被拆错；shell 自身的状态变量被排除。
    assert_eq!(get("MULTI").as_deref(), Some("a\nb"));
    assert_eq!((get("PWD"), get("SHLVL")), (None, None));
    assert_eq!(vars.len(), 3);

    assert!(parse(b"no markers").is_none());
    assert!(
        parse(format!("{MARKER}A=1\0{MARKER}").as_bytes()).is_none(),
        "没有 PATH 不算数"
    );
}

#[cfg(unix)]
#[test]
fn a_fake_login_shell_is_asked_and_waited_for() {
    // 假的 shell：像 `.zshrc` 那样先打印几行，再用 `sh` 执行传入的脚本，避免运行用户
    // 真正的 shell。
    let dir = scratch("environment");
    let shell = fake_tool(
        &dir,
        "shell",
        "echo 'Last login: today'\nexec /bin/sh -c \"$4\"",
    );
    // 这里不测超时（下面的 `stuck` 测），按模拟工具给足时间。
    let vars = login_shell(shell.as_os_str(), TOOL_TIMEOUT).unwrap();
    let get = |name: &str| vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone());
    assert_eq!(get("PATH"), std::env::var_os("PATH"));
    assert_eq!(get("WINC_RESOLVING_ENVIRONMENT"), None);

    // 一直不退出的 shell（例如 `.zshrc` 在等待输入，或用 `exec` 启动了交互式程序）：
    // 到时间即放弃。
    let stuck = fake_tool(&dir, "stuck", "sleep 30");
    let started = std::time::Instant::now();
    assert!(login_shell(stuck.as_os_str(), Duration::from_millis(300)).is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    // 只打印了其他内容，没有打印环境：应返回错误。
    let odd = fake_tool(&dir, "odd", "echo hello");
    assert!(login_shell(odd.as_os_str(), SHELL_TIMEOUT).is_err());
}

/// 回归：没有 `SHELL` 变量时固定用 `/bin/zsh`，没装 zsh 的系统上取不到环境。
#[cfg(unix)]
#[test]
fn the_account_names_the_login_shell() {
    let shell = PathBuf::from(account_shell().expect("当前用户有登录 shell"));
    assert!(shell.is_absolute() && shell.exists(), "{}", shell.display());
}
