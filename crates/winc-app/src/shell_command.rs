//! 命令行入口：让 `winc .` 在终端里可用。
//!
//! Windows 安装包和 deb 包已经把 `winc` 加入 `PATH`。macOS 的程序位于 `.app` 内，AppImage 没有固定
//! 路径，由用户执行命令后创建符号链接（[`placement`]）。启动时不自动安装：写入系统目录必须由用户
//! 发起。
//!
//! 在 Unix 上从终端启动时，窗口进程转到后台（[`detach`]），命令立即返回。

use crate::cli::Args;

/// 符号链接的位置和它指向的程序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    /// 符号链接的路径。
    pub link: std::path::PathBuf,
    /// 链接指向的程序。
    pub target: std::path::PathBuf,
}

/// 安装或卸载的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// 已创建链接。
    Installed,
    /// 已移除链接。
    Uninstalled,
    /// 卸载时没有链接。
    NotInstalled,
    /// 链接位置上是其他程序，没有改动。
    Occupied,
    /// 没有写入权限，需要授权。
    Denied,
    /// 其他失败，带系统的错误说明。
    Failed(String),
}

/// 需要授权的安装或卸载结束时送回主线程的结果。
#[derive(Debug)]
pub(crate) struct Done {
    /// 操作的链接。
    pub placement: Placement,
    /// 结果；用户取消授权时为 `None`。
    pub outcome: Option<Outcome>,
}

/// 返回当前运行方式下要创建的链接；Windows、deb 包和开发构建无需安装，返回 `None`。
pub(crate) fn placement() -> Option<Placement> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
        macos_placement(&exe)
    }
    #[cfg(target_os = "linux")]
    {
        let appimage = std::env::var_os("APPIMAGE")?;
        let home = std::env::var_os("HOME")?;
        appimage_placement(appimage.as_ref(), home.as_ref())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// macOS：程序位于 `.app` 内时链接到 `/usr/local/bin/winc`，该目录在系统默认的 `PATH`（`/etc/paths`）
/// 中。
#[cfg(any(target_os = "macos", test))]
fn macos_placement(exe: &std::path::Path) -> Option<Placement> {
    let dir = exe.parent()?;
    let bundled = dir.ends_with("Contents/MacOS")
        && dir
            .ancestors()
            .nth(2)?
            .extension()
            .is_some_and(|e| e == "app");
    bundled.then(|| Placement {
        link: "/usr/local/bin/winc".into(),
        target: exe.to_path_buf(),
    })
}

/// AppImage：链接到用户目录的 `~/.local/bin/winc`（XDG 约定的用户可执行文件目录），不需要授权。
#[cfg(any(target_os = "linux", test))]
fn appimage_placement(appimage: &std::path::Path, home: &std::path::Path) -> Option<Placement> {
    (appimage.is_absolute() && home.is_absolute()).then(|| Placement {
        link: home.join(".local/bin/winc"),
        target: appimage.to_path_buf(),
    })
}

/// 链接指向的 `found` 是否由 winc 创建；只有这种链接可以替换或移除。
///
/// 目标已不存在的链接同样视为可替换，这类链接是应用移动后的遗留。
#[cfg(unix)]
fn is_ours(placement: &Placement, found: &std::path::Path) -> bool {
    let bundled = found.ends_with("Contents/MacOS/winc");
    let appimage = found.extension().is_some_and(|e| e == "AppImage");
    found == placement.target || bundled || appimage || !found.exists()
}

/// 创建链接；已有 winc 创建的链接时替换。先创建临时链接再改名，失败时原链接不受影响。
#[cfg(unix)]
pub(crate) fn install(placement: &Placement) -> Outcome {
    let link = &placement.link;
    match std::fs::symlink_metadata(link) {
        Ok(meta) if meta.is_symlink() => match std::fs::read_link(link) {
            Ok(found) if is_ours(placement, &found) => {}
            _ => return Outcome::Occupied,
        },
        Ok(_) => return Outcome::Occupied,
        Err(_) => {}
    }
    let temporary = link.with_file_name(".winc.tmp");
    let create = || {
        if let Some(dir) = link.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&temporary);
        std::os::unix::fs::symlink(&placement.target, &temporary)?;
        std::fs::rename(&temporary, link)
    };
    match create() {
        Ok(()) => Outcome::Installed,
        Err(e) => failure(&e),
    }
}

/// 移除链接；只移除 winc 创建的链接。
#[cfg(unix)]
pub(crate) fn uninstall(placement: &Placement) -> Outcome {
    let link = &placement.link;
    match std::fs::symlink_metadata(link) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Outcome::NotInstalled,
        Err(e) => return failure(&e),
        Ok(meta) if !meta.is_symlink() => return Outcome::Occupied,
        Ok(_) => {}
    }
    match std::fs::read_link(link) {
        Ok(found) if is_ours(placement, &found) => {}
        _ => return Outcome::Occupied,
    }
    match std::fs::remove_file(link) {
        Ok(()) => Outcome::Uninstalled,
        Err(e) => failure(&e),
    }
}

#[cfg(unix)]
fn failure(error: &std::io::Error) -> Outcome {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            Outcome::Denied
        }
        _ => Outcome::Failed(error.to_string()),
    }
}

/// 非 Unix 平台的 [`placement`] 恒为 `None`，不会调用此函数。
#[cfg(not(unix))]
pub(crate) fn install(_: &Placement) -> Outcome {
    Outcome::Failed(String::new())
}

/// 非 Unix 平台的 [`placement`] 恒为 `None`，不会调用此函数。
#[cfg(not(unix))]
pub(crate) fn uninstall(_: &Placement) -> Outcome {
    Outcome::Failed(String::new())
}

/// 返回用单引号包裹的路径，shell 将其读作一个参数。
#[cfg(any(target_os = "macos", test))]
fn shell_quoted(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// 返回通过系统授权对话框执行 `command` 的 AppleScript。
#[cfg(any(target_os = "macos", test))]
fn authorized_script(command: &str) -> String {
    let escaped = command.replace('\\', r"\\").replace('"', "\\\"");
    format!("do shell script \"{escaped}\" with administrator privileges")
}

/// 返回需要授权时执行的 shell 命令：`installing` 为真时创建链接，否则移除。
#[cfg(any(target_os = "macos", test))]
fn authorized_command(placement: &Placement, installing: bool) -> String {
    let link = shell_quoted(&placement.link);
    if !installing {
        return format!("/bin/rm -f {link}");
    }
    let target = shell_quoted(&placement.target);
    let dir = placement.link.parent().unwrap_or(&placement.link);
    let dir = shell_quoted(dir);
    format!("/bin/mkdir -p {dir} && /bin/ln -sfn {target} {link}")
}

/// 显示系统授权对话框并执行安装或卸载。调用会阻塞到用户响应，必须在后台线程调用。
///
/// 用户取消时返回 `None`。
#[cfg(target_os = "macos")]
pub(crate) fn run_authorized(placement: &Placement, installing: bool) -> Option<Outcome> {
    /// `osascript` 报告的「用户已取消」错误码（AppleScript 的 `userCanceledErr`）。
    const USER_CANCELED: &str = "(-128)";
    let script = authorized_script(&authorized_command(placement, installing));
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output();
    match output {
        Ok(output) if output.status.success() => Some(if installing {
            Outcome::Installed
        } else {
            Outcome::Uninstalled
        }),
        Ok(output) => {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            (!message.contains(USER_CANCELED)).then_some(Outcome::Failed(message))
        }
        Err(e) => Some(Outcome::Failed(e.to_string())),
    }
}

/// 通过符号链接启动时切换到程序的真实路径，参数和标准输入输出不变。
///
/// macOS 报告的可执行文件路径不解析符号链接，程序要由真实路径定位应用包和同目录的组件。切换失败时
/// 继续运行。
pub(crate) fn enter_real_path() {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Ok(real) = exe.canonicalize() else {
            return;
        };
        if real != exe {
            let _ = std::process::Command::new(real)
                .args(std::env::args_os().skip(1))
                .exec();
        }
    }
}

/// 从终端启动时，用同样的参数在后台启动窗口进程；已启动时返回真，调用方应当退出。
///
/// 仅在标准输入是终端时转移；新进程的标准输入不是终端，不会再次转移。`--wait` 和 `-` 必须留在前台，
/// `--new-instance` 和开发构建留在前台以便查看输出。
pub(crate) fn detach(args: &Args) -> bool {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        if cfg!(debug_assertions) || stays_in_foreground(args) || !std::io::stdin().is_terminal() {
            return false;
        }
        let Some(program) = background_program() else {
            return false;
        };
        // 独立的进程组：终端关闭时发给前台进程组的 SIGHUP 不会结束窗口进程。
        Command::new(program)
            .args(std::env::args_os().skip(1))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .is_ok()
    }
    #[cfg(not(unix))]
    {
        // Windows 的程序属于图形子系统，从命令行启动时命令已经立即返回。
        let _ = args;
        false
    }
}

/// 这次启动是否必须留在前台。
#[cfg(unix)]
fn stays_in_foreground(args: &Args) -> bool {
    args.one_off() || args.new_instance
}

/// 返回后台启动的程序。
///
/// AppImage 的文件系统在启动进程退出时卸载，必须重新运行 AppImage 本身。
#[cfg(unix)]
fn background_program() -> Option<std::path::PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(Into::into)
        .or_else(|| std::env::current_exe().ok())
}

#[cfg(all(test, unix))]
mod tests;
