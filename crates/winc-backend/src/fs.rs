//! 文件操作：函数名和语义与 `std::fs` 中的同名函数相同，路径可以是本机路径或虚拟路径。
//!
//! 目前只有本机的实现；虚拟路径一律返回 [`io::ErrorKind::Unsupported`]，不会交给本机的
//! 文件系统。文件信息用本 crate 的 [`Metadata`] 表示：`std::fs::Metadata` 无法由远端的数据构造。

#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ensure_local;

/// 文件的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 普通文件。
    File,
    /// 目录。
    Dir,
    /// 符号链接；只出现在 [`symlink_metadata`] 的结果中。
    Symlink,
    /// 其他：设备、管道、套接字等。
    Other,
}

/// 修改时间：自 Unix 纪元起的秒数和秒内的纳秒数（0 到 999 999 999），纪元之前秒数为负。
///
/// 只用于判断文件是否发生变化，按原样保存和比较，不换算为本机的时间类型：各系统时间类型的
/// 精度不同，换算会丢失远端文件系统的纳秒部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Stamp {
    /// 自 Unix 纪元起的秒数。
    pub secs: i64,
    /// 秒内的纳秒数。
    pub nanos: u32,
}

impl From<SystemTime> for Stamp {
    fn from(time: SystemTime) -> Self {
        match time.duration_since(UNIX_EPOCH) {
            Ok(after) => Self {
                secs: after.as_secs().try_into().unwrap_or(i64::MAX),
                nanos: after.subsec_nanos(),
            },
            // 纪元之前：秒数向下取整，纳秒部分保持非负。
            Err(error) => {
                let before = error.duration();
                let secs: i64 = before.as_secs().try_into().unwrap_or(i64::MAX);
                if before.subsec_nanos() == 0 {
                    Self {
                        secs: -secs,
                        nanos: 0,
                    }
                } else {
                    Self {
                        secs: -secs - 1,
                        nanos: 1_000_000_000 - before.subsec_nanos(),
                    }
                }
            }
        }
    }
}

/// Unix 系统上的文件属性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unix {
    /// 文件类型与权限位（`st_mode`）。
    pub mode: u32,
    /// 属主的用户编号。
    pub uid: u32,
    /// 硬链接数。
    pub nlink: u64,
    /// 所在设备的编号；与 `ino` 一起标识同一个文件。
    pub dev: u64,
    /// inode 编号。
    pub ino: u64,
}

/// 文件信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    /// 种类。
    pub kind: Kind,
    /// 大小（字节）。
    pub len: u64,
    /// 修改时间；文件系统不提供时为 `None`。
    pub modified: Option<Stamp>,
    /// 是否只读：Unix 上所有写权限位均未设置，Windows 上设置了只读属性。
    pub readonly: bool,
    /// Unix 系统上的属性；文件位于 Windows 时为 `None`。
    pub unix: Option<Unix>,
}

impl Metadata {
    /// 是否为普通文件。
    pub fn is_file(&self) -> bool {
        self.kind == Kind::File
    }

    /// 是否为目录。
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    /// 是否为符号链接。
    pub fn is_symlink(&self) -> bool {
        self.kind == Kind::Symlink
    }
}

impl From<&std::fs::Metadata> for Metadata {
    fn from(metadata: &std::fs::Metadata) -> Self {
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Symlink
        } else if file_type.is_dir() {
            Kind::Dir
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        #[cfg(unix)]
        let unix = {
            use std::os::unix::fs::MetadataExt;
            Some(Unix {
                mode: metadata.mode(),
                uid: metadata.uid(),
                nlink: metadata.nlink(),
                dev: metadata.dev(),
                ino: metadata.ino(),
            })
        };
        #[cfg(not(unix))]
        let unix = None;
        Self {
            kind,
            len: metadata.len(),
            modified: metadata.modified().ok().map(Stamp::from),
            readonly: metadata.permissions().readonly(),
            unix,
        }
    }
}

/// 返回 `path` 的文件信息，跟随符号链接。
///
/// # Errors
///
/// 与 `std::fs::metadata` 相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref();
    ensure_local(path)?;
    std::fs::metadata(path).map(|m| Metadata::from(&m))
}

/// 返回 `path` 的文件信息，不跟随符号链接。
///
/// # Errors
///
/// 与 `std::fs::symlink_metadata` 相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref();
    ensure_local(path)?;
    std::fs::symlink_metadata(path).map(|m| Metadata::from(&m))
}

/// 是否存在并且是普通文件（跟随符号链接）；无法确定时为否。
pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    metadata(path).is_ok_and(|m| m.is_file())
}

/// 是否存在并且是目录（跟随符号链接）；无法确定时为否。
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    metadata(path).is_ok_and(|m| m.is_dir())
}

/// 是否为符号链接（不跟随链接，指向不存在目标的链接同样为是）；无法确定时为否。
pub fn is_symlink(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    symlink_metadata(path).is_ok_and(|m| m.is_symlink())
}

/// 是否存在（跟随符号链接，指向不存在目标的符号链接不算存在）；无法确定时为否。
pub fn exists(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    metadata(path).is_ok()
}

/// 读取 `path` 的全部内容。
///
/// # Errors
///
/// 与 `std::fs::read` 相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    ensure_local(path)?;
    std::fs::read(path)
}

/// 读取 `path` 的全部内容并按 UTF-8 解码。
///
/// # Errors
///
/// 与 `std::fs::read_to_string` 相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    ensure_local(path)?;
    std::fs::read_to_string(path)
}

/// 返回符号链接 `path` 指向的目标。
///
/// # Errors
///
/// 与 `std::fs::read_link` 相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let path = path.as_ref();
    ensure_local(path)?;
    std::fs::read_link(path)
}
