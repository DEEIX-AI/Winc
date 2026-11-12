//! 打开文件用于读取：文件信息、开头的一段和内容取自同一次打开。

#[cfg(test)]
mod tests;

use std::io::{self, Read, Seek};
use std::path::Path;

use super::Metadata;
use crate::ensure_local;

/// 打开的文件：文件信息、开头的一段，以及读取全部内容或换成 [`Handle`] 的入口。
///
/// 三者取自同一次打开，其间文件被其他程序改名替换时读到的仍是打开时的那个文件。
pub struct Opened {
    /// 打开时的文件信息。
    pub metadata: Metadata,
    /// 文件开头的若干字节，长度不超过 [`open`] 的 `sniff`；文件更短时是全部内容。
    pub head: Vec<u8>,
    file: std::fs::File,
}

impl Opened {
    /// 返回从文件开头顺序读取内容的读取器；每次调用都从开头开始。
    ///
    /// # Errors
    ///
    /// 无法回到文件开头时返回对应的 `io::Error`。
    pub fn reader(&mut self) -> io::Result<impl Read + '_> {
        self.file.rewind()?;
        Ok(&mut self.file)
    }

    /// 换成可按偏移读取的句柄。
    pub fn into_handle(self) -> Handle {
        Handle(self.file)
    }
}

/// 可按偏移读取的文件句柄。
///
/// 句柄存在期间，文件被改名替换后读到的仍是打开时的内容；文件被原地修改时读到修改后的内容，
/// 用 [`Handle::metadata`] 发现。可以在多个线程上同时读取。
#[derive(Debug)]
pub struct Handle(std::fs::File);

impl Handle {
    /// 返回句柄指向的文件此刻的信息（不经过路径，文件改名后仍是同一个文件）。
    ///
    /// # Errors
    ///
    /// 与 `std::fs::File::metadata` 相同。
    pub fn metadata(&self) -> io::Result<Metadata> {
        self.0.metadata().map(|m| Metadata::from(&m))
    }

    /// 从字节偏移 `offset` 处读取，至多填满 `buf`，返回读到的字节数；到达文件末尾时返回 0。
    ///
    /// # Errors
    ///
    /// 与系统的按偏移读取（Unix 的 `pread`，Windows 的 `seek_read`）相同，包括被信号中断。
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        return std::os::unix::fs::FileExt::read_at(&self.0, buf, offset);
        #[cfg(windows)]
        return std::os::windows::fs::FileExt::seek_read(&self.0, buf, offset);
    }

    /// 复制句柄，两个句柄指向同一个打开的文件。
    ///
    /// # Errors
    ///
    /// 与 `std::fs::File::try_clone` 相同。
    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }
}

impl From<std::fs::File> for Handle {
    fn from(file: std::fs::File) -> Self {
        Self(file)
    }
}

/// 打开 `path` 用于读取，并读出开头至多 `sniff` 字节。
///
/// # Errors
///
/// 与 `std::fs::File::open` 和读取相同；`path` 是虚拟路径时返回 [`io::ErrorKind::Unsupported`]。
pub fn open(path: impl AsRef<Path>, sniff: usize) -> io::Result<Opened> {
    let path = path.as_ref();
    ensure_local(path)?;
    let mut file = std::fs::File::open(path)?;
    let metadata = Metadata::from(&file.metadata()?);
    let mut head = Vec::with_capacity(sniff);
    (&mut file).take(sniff as u64).read_to_end(&mut head)?;
    Ok(Opened {
        metadata,
        head,
        file,
    })
}
