//! 列目录。

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::Kind;
use crate::ensure_local;

/// 目录中的一项。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 名字，不含目录。
    pub name: OsString,
    /// 种类，不跟随符号链接：符号链接本身为 [`Kind::Symlink`]。
    pub kind: Kind,
    dir: Arc<Path>,
}

impl Entry {
    /// 返回该项的完整路径：所列的目录加上名字。
    pub fn path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }
}

/// [`read_dir`] 返回的迭代器，逐项给出目录的直接子项，顺序不确定，不含 `.` 和 `..`。
///
/// 读取某一项或取其种类失败时该项为错误，其余各项不受影响。
pub struct ReadDir {
    dir: Arc<Path>,
    entries: std::fs::ReadDir,
}

impl Iterator for ReadDir {
    type Item = io::Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        Some(entry.and_then(|entry| {
            Ok(Entry {
                name: entry.file_name(),
                kind: entry.file_type()?.into(),
                dir: Arc::clone(&self.dir),
            })
        }))
    }
}

/// 列出目录 `path` 的直接子项。
///
/// # Errors
///
/// 与 `std::fs::read_dir` 相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    let path = path.as_ref();
    ensure_local(path)?;
    Ok(ReadDir {
        entries: std::fs::read_dir(path)?,
        dir: Arc::from(path),
    })
}
