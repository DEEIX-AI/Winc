//! 取回网络上的小文件：运行系统的 `curl`，未安装时运行 `wget`，本程序不链接网络库。
//!
//! 应用内更新读取 appcast、读取更新日志都经过这里。

use std::io;
use std::process::{Command, Stdio};

/// 构造运行 `program` 的命令。
#[cfg(unix)]
pub(crate) fn command(program: &str) -> Command {
    Command::new(program)
}

/// 构造运行 `program` 的命令，不为它创建控制台窗口。
#[cfg(windows)]
pub(crate) fn command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NO_WINDOW`：不为控制台程序创建窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// 取回 `url` 的内容，最长等待 `timeout` 秒：优先使用 `curl`，未安装时使用 `wget`。
///
/// 请求失败时返回 `None`。
pub(crate) fn fetch(url: &str, timeout: u64) -> Option<Vec<u8>> {
    let timeout = timeout.to_string();
    let curl = command("curl")
        .args([
            "--fail",
            "--silent",
            "--location",
            "--max-time",
            &timeout,
            url,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let output = match curl {
        Err(e) if e.kind() == io::ErrorKind::NotFound => command("wget")
            .args([
                "--quiet",
                "--timeout",
                &timeout,
                "--output-document",
                "-",
                url,
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?,
        result => result.ok()?,
    };
    output.status.success().then_some(output.stdout)
}
