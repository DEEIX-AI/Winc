//! 改动文件系统的操作：建目录、新建文件、改名、复制、删除。
//!
//! 函数名和语义与 `std::fs` 中的同名函数相同。虚拟路径上的操作在远端执行，此时还可能返回
//! 连接的错误（见 [`fs`](crate::fs) 的说明）。涉及两个路径的操作要求两者在同一台机器上，否则
//! 返回 [`io::ErrorKind::Unsupported`]。新建文件和覆盖写入的内容连同路径必须放进一帧（约 1 MiB）。

#[cfg(test)]
mod tests;

use std::io::{self, Write};
use std::path::Path;

use crate::{locate, remote};

/// 创建目录 `path`；上级目录必须已存在。
///
/// # Errors
///
/// 与 `std::fs::create_dir` 相同。
pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::create_dir(path),
        Some(at) => remote::connection(&at.target)?.create_dir(&at.path, false),
    }
}

/// 创建目录 `path` 及其不存在的各级上级目录；目录已存在时成功。
///
/// # Errors
///
/// 与 `std::fs::create_dir_all` 相同。
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::create_dir_all(path),
        Some(at) => remote::connection(&at.target)?.create_dir(&at.path, true),
    }
}

/// 新建文件 `path` 并写入 `contents`；`path` 上已有任何条目时不覆盖。
///
/// 写入失败时删除写了一部分的文件。
///
/// # Errors
///
/// `path` 已存在时返回 [`io::ErrorKind::AlreadyExists`]；其余与创建和写入文件相同。
pub fn create_new(path: impl AsRef<Path>, contents: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(at) = locate(path) {
        return remote::connection(&at.target)?.create_new(&at.path, contents);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if let Err(error) = file.write_all(contents) {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

/// 把 `contents` 写入 `path`，文件已存在时覆盖全部内容。
///
/// 不经过临时文件，写入中途失败时文件只剩部分内容；文档的保存使用 [`save`](super::save)。
///
/// # Errors
///
/// 与 `std::fs::write` 相同。
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::write(path, contents),
        Some(at) => remote::connection(&at.target)?.write(&at.path, contents.as_ref()),
    }
}

/// 把 `from` 改名为 `to`。
///
/// # Errors
///
/// 与 `std::fs::rename` 相同。
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    match remote::pair(from, to)? {
        None => std::fs::rename(from, to),
        Some((connection, from, to)) => connection.rename(&from, &to),
    }
}

/// 把文件 `from` 的内容和权限复制到 `to`，返回复制的字节数；`to` 已存在时覆盖。
///
/// # Errors
///
/// 与 `std::fs::copy` 相同。
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    let (from, to) = (from.as_ref(), to.as_ref());
    match remote::pair(from, to)? {
        None => std::fs::copy(from, to),
        Some((connection, from, to)) => connection.copy(&from, &to),
    }
}

/// 把符号链接 `from` 复制为指向同一目标的符号链接 `to`，不复制目标的内容。
///
/// Windows 上链接分为指向目录的和指向文件的两种，按 `from` 此刻指向的目标选择；创建符号链接
/// 需要相应的权限。
///
/// # Errors
///
/// 读取或创建链接失败时返回对应的 `io::Error`。
pub fn copy_symlink(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    if let Some((connection, from, to)) = remote::pair(from, to)? {
        return connection.copy_symlink(&from, &to);
    }
    let target = std::fs::read_link(from)?;
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, to);
    #[cfg(windows)]
    return if std::fs::metadata(from).is_ok_and(|m| m.is_dir()) {
        std::os::windows::fs::symlink_dir(target, to)
    } else {
        std::os::windows::fs::symlink_file(target, to)
    };
}

/// 删除文件 `path`（符号链接删除链接本身）。
///
/// # Errors
///
/// 与 `std::fs::remove_file` 相同。
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match locate(path) {
        None => std::fs::remove_file(path),
        Some(at) => remote::connection(&at.target)?.remove_file(&at.path),
    }
}
