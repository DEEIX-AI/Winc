//! WSL：Windows 上通过 `\\wsl.localhost\<发行版>\...` 访问的目录。
//!
//! 这些目录中的文件由 Windows 读写，Git 和终端在对应的发行版内运行。本模块负责两种路径写法之间的
//! 换算。换算本身不依赖平台；仅在 Windows 上生效的入口在各自的文档中说明。

use std::path::{Path, PathBuf};

/// 访问发行版文件系统的主机名：`wsl.localhost` 是当前的写法，`wsl$` 是早期的写法，两者均可用。
const HOSTS: [&str; 2] = ["wsl.localhost", "wsl$"];

/// 通过 `WSLENV` 传给发行版内 `git` 的环境变量：`wsl.exe` 默认不向发行版传递环境变量。
pub(crate) const GIT_ENV: &str =
    "GIT_OPTIONAL_LOCKS:GIT_TERMINAL_PROMPT:GIT_EDITOR:GIT_INDEX_FILE:NO_COLOR:LC_ALL:LANGUAGE";

/// 发行版中的一个位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 路径中的主机名。换算回 Windows 路径时沿用，使同一仓库的路径可以相互比较。
    host: String,
    /// 发行版名称。
    pub distro: String,
    /// 发行版内的绝对路径。
    pub path: String,
}

/// 返回 `path` 对应的发行版位置；不是发行版中的路径时返回 `None`。
///
/// 接受 `\\?\UNC\` 前缀和两种斜杠，主机名不区分大小写。
#[must_use]
pub fn locate(path: &Path) -> Option<Location> {
    let text = path.to_string_lossy().replace('/', "\\");
    let rest = text
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| text.strip_prefix(r"\\"))?;
    let mut parts = rest.split('\\').filter(|part| !part.is_empty());
    let host = parts.next()?;
    let host = HOSTS.iter().find(|h| h.eq_ignore_ascii_case(host))?;
    let distro = parts.next()?.to_owned();
    let path = parts.fold(String::new(), |path, part| path + "/" + part);
    Some(Location {
        host: (*host).to_owned(),
        distro,
        path: if path.is_empty() {
            "/".to_owned()
        } else {
            path
        },
    })
}

/// 同 [`locate`]，但仅在 Windows 上生效：其他平台上这种写法不表示发行版中的路径。
#[must_use]
pub fn locate_here(path: &Path) -> Option<Location> {
    if cfg!(windows) { locate(path) } else { None }
}

/// 把 `git` 输出的路径 `text` 转换为本机路径。
///
/// `root` 位于发行版中时，绝对路径按同一发行版内的路径处理；其余路径相对于 `root`。
#[must_use]
pub fn resolve(root: &Path, text: &str) -> PathBuf {
    match locate_here(root).filter(|_| text.starts_with('/')) {
        Some(place) => place.windows_path(text),
        None => root.join(text),
    }
}

/// 返回传给 `git` 的路径参数：`root` 和 `path` 都位于发行版中时使用发行版内的路径。
#[must_use]
pub fn git_arg(root: &Path, path: &Path) -> String {
    match locate_here(root).and(locate_here(path)) {
        Some(place) => place.path,
        None => path.to_string_lossy().into_owned(),
    }
}

/// 返回存放 `git` 临时文件的目录。
///
/// `root` 位于发行版中时使用发行版的 `/tmp`：发行版内的 `git` 可以直接访问，且不依赖 Windows 磁盘的
/// 自动挂载。其余情况使用系统的临时目录。
#[must_use]
pub fn temp_dir(root: &Path) -> PathBuf {
    match locate_here(root) {
        Some(place) => place.windows_path("/tmp"),
        None => std::env::temp_dir(),
    }
}

/// 本机是否已安装 WSL。只检查 `wsl.exe` 是否存在，不启动它。
#[must_use]
pub fn available() -> bool {
    cfg!(windows)
        && std::env::var_os("SystemRoot")
            .is_some_and(|root| Path::new(&root).join(r"System32\wsl.exe").is_file())
}

impl Location {
    /// 返回本发行版内的绝对路径 `linux` 在 Windows 上的访问路径。
    #[must_use]
    pub fn windows_path(&self, linux: &str) -> PathBuf {
        let mut text = format!(r"\\{}\{}", self.host, self.distro);
        for part in linux.split('/').filter(|part| !part.is_empty()) {
            text.push('\\');
            text.push_str(part);
        }
        PathBuf::from(text)
    }
}

#[cfg(test)]
mod tests;
