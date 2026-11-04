//! 配置、状态和文档共用的文件操作：winc 的配置目录、原子写入用的临时文件和落盘。

use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// 返回 winc 的配置目录。
///
/// 各平台的路径规则与 [`config_path`](crate::config_path) 相同；无法确定用户目录时返回 `None`。
pub(crate) fn config_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);

    if cfg!(target_os = "macos") {
        home().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("winc")
        })
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(home)
            .map(|dir| dir.join("winc"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".config")))
            .map(|dir| dir.join("winc"))
    }
}

/// 规范化路径：解开符号链接、`.` 和 `..`，规则同 [`std::fs::canonicalize`]。
///
/// Windows 上 [`std::fs::canonicalize`] 返回带 `\\?\` 前缀的路径：该前缀显示出来不直观，其他程序
/// 可能不认。不需要该前缀也能表示时去掉它，规则见 `simplify_verbatim`。
///
/// # Errors
///
/// 路径不存在或无法读取时返回对应的 `std::io::Error`。
#[allow(clippy::disallowed_methods)] // 唯一直接调用 `std::fs::canonicalize` 的地方
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let path = std::fs::canonicalize(path)?;
    Ok(if cfg!(windows) {
        simplify_verbatim(path)
    } else {
        path
    })
}

/// 去掉 Windows 路径中不必要的 `\\?\` 前缀，见 [`canonicalize`]；去掉后含义可能改变时原样返回。
///
/// 网络路径换回 `\\服务器\共享`，盘符开头的路径去掉前缀；去掉后的路径不超过 `MAX_PATH`、各段不
/// 以点或空格结尾时才去掉。
fn simplify_verbatim(path: PathBuf) -> PathBuf {
    /// 不带 `\\?\` 前缀时路径的最大长度（`MAX_PATH`，含结尾的 NUL）。
    const MAX_PATH: usize = 260;
    let Some(text) = path.to_str() else {
        return path;
    };
    let plain = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let drive = rest.as_bytes();
        if drive.len() < 3
            || !drive[0].is_ascii_alphabetic()
            || drive[1] != b':'
            || drive[2] != b'\\'
        {
            return path;
        }
        rest.to_owned()
    } else {
        return path;
    };
    let awkward = plain
        .split('\\')
        .any(|part| part.ends_with('.') || part.ends_with(' '));
    if plain.len() >= MAX_PATH || awkward {
        return path;
    }
    PathBuf::from(plain)
}

/// 原子地写入 `path`：先写同目录下的临时文件并落盘，再改名覆盖，中途断电不会留下半截文件。
///
/// `path` 是符号链接时写到链接指向的文件，链接本身保留；文件已存在时新文件沿用原来的权限。
/// 目录不存在时先创建；失败时删除临时文件。
///
/// # Errors
///
/// 目录创建失败、临时文件写入或落盘失败、改名失败时返回对应的 `std::io::Error`。
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let path = canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or(Cow::Borrowed("file"), |n| n.to_string_lossy());
    let existing = std::fs::metadata(&path).ok();
    let (temp, mut file) = create_temporary(dir, &name, existing.is_some())?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        sync_file(&file)?;
        if let Some(metadata) = &existing {
            file.set_permissions(metadata.permissions())?;
        }
        replace_file(&temp, &path)?;
        sync_dir(dir);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// 把写好的临时文件 `temp` 改名为 `path`，替换原有的文件。
///
/// Windows 上目标正被打开时直接改名会报拒绝访问：先每隔几毫秒重试，累计约一秒；文件一直被
/// 打开着、文件系统又不支持 POSIX 式的替换（FAT、exFAT）时，改为把目标移开再换上新文件。
///
/// # Errors
///
/// 改名失败（Windows 上重试和移开都不行）时返回对应的 `std::io::Error`。
pub fn replace_file(temp: &Path, path: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(temp, path) {
            Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
                if tries >= 20 {
                    return replace_aside(temp, path).map_err(|_| e);
                }
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(5 * tries));
            }
            result => return result,
        }
    }
}

/// 先把 `path` 改名到同一目录下的临时名字，再把 `temp` 改名为 `path`，最后删掉移开的旧文件。
///
/// 打开时允许删除的文件可以改名，但不能被替换。换上新文件失败时把旧文件移回原处。旧文件被别的
/// 程序短暂打开着时删除会失败，稍等再试；仍删不掉时标为关闭后删除（[`delete_on_close`]），等
/// 其他打开该文件的进程都关掉以后由系统删除。
fn replace_aside(temp: &Path, path: &Path) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or(Cow::Borrowed("file"), |n| n.to_string_lossy());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let aside = dir.join(format!(".{name}.{}-{stamp}.old", std::process::id()));
    std::fs::rename(path, &aside)?;
    if let Err(e) = std::fs::rename(temp, path) {
        let _ = std::fs::rename(&aside, path);
        return Err(e);
    }
    for tries in 1..=10 {
        if std::fs::remove_file(&aside).is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(5 * tries));
    }
    delete_on_close(&aside);
    Ok(())
}

/// 把 `path` 标为关闭后删除：打开一次（带 `FILE_FLAG_DELETE_ON_CLOSE`）随即关掉，其他打开该
/// 文件的句柄都关闭以后由系统删除。
///
/// 只在 Windows 上有这种标记，其他平台上什么也不做。
fn delete_on_close(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        /// `FILE_FLAG_DELETE_ON_CLOSE`。
        const DELETE_ON_CLOSE: u32 = 0x0400_0000;
        /// `DELETE` 访问权限。
        const DELETE: u32 = 0x0001_0000;
        /// 允许别人读、写、删除（`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`）。
        const SHARE_ALL: u32 = 0x7;
        let _ = std::fs::OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(SHARE_ALL)
            .custom_flags(DELETE_ON_CLOSE)
            .open(path);
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// 在 `dir` 里为名为 `name` 的文件新建原子写入用的临时文件，返回路径和打开的文件。
///
/// 临时文件名带随机后缀，以 `create_new` 只新建、不跟随符号链接：目录别人也能写时，对方预先放
/// 一个同名的符号链接就能让写入落到链接指向的文件。`private` 为 `true` 时只给写者读写（0600）。
///
/// # Errors
///
/// 创建失败、或重试几次名字都已被占用时返回对应的 `std::io::Error`。
pub fn create_temporary(
    dir: &Path,
    name: &str,
    private: bool,
) -> std::io::Result<(PathBuf, std::fs::File)> {
    use std::hash::{BuildHasher, RandomState};
    use std::io::ErrorKind;

    /// 临时文件名里 `name` 最多占的字节数：255 减去前后加上的 27 字节。
    const NAME_BYTES: usize = 255 - 27;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;

    let mut end = name.len().min(NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let name = &name[..end];
    // `RandomState` 每次创建都取一个随机的种子，不需要另外引入随机数的库。
    let random = RandomState::new();
    let mut last = None;
    for attempt in 0..8u32 {
        let suffix = random.hash_one((std::process::id(), attempt));
        let temp = dir.join(format!(".{name}.winc-{suffix:016x}.tmp"));
        match options.open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::from(ErrorKind::AlreadyExists)))
}

/// 把文件内容写入磁盘，保证其后的 `rename` 不会先于文件内容持久化。
///
/// macOS 上标准库的 `sync_all` 使用 `F_FULLFSYNC`，要等磁盘控制器清空写缓存，外置磁盘和网络卷上
/// 明显更慢，因此改用只保证写入顺序的 `F_BARRIERFSYNC`：断电后磁盘上是完整的旧文件或新文件。
///
/// # Errors
///
/// `sync_all` 失败时返回对应的 `io::Error`。
#[cfg(target_vendor = "apple")]
// 本函数只有对 `libc::fcntl` 的一次调用需要 `unsafe`，因此只在此处放开 `unsafe_code`。
#[allow(unsafe_code)]
pub fn sync_file(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: 参数是 `file` 持有的有效文件描述符和整数命令，不涉及指针；`file` 在调用期间被借用，
    // 描述符不会被关闭。调用失败时只返回 -1。
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_BARRIERFSYNC) } == 0 {
        return Ok(());
    }
    file.sync_all()
}

/// 把文件内容写入磁盘，保证其后的 `rename` 不会先于文件内容持久化。
///
/// # Errors
///
/// `sync_all` 失败时返回对应的 `io::Error`。
#[cfg(not(target_vendor = "apple"))]
pub fn sync_file(file: &std::fs::File) -> std::io::Result<()> {
    file.sync_all()
}

/// 把目录 `dir` 本身同步到磁盘：改名（`rename`）只改了目录项，文件内容 `sync_all` 过了，目录项
/// 却可能还在缓存里，这时断电，重启后看到的仍是改名前的旧文件。
///
/// 尽力而为：部分平台和文件系统不支持同步目录，出错时忽略，写入本身已经成功。macOS 上不执行：
/// 目录的 `sync_all` 同样使用 `F_FULLFSYNC`，而文件内容已由 [`sync_file`] 保证先于改名持久化。
pub fn sync_dir(dir: &Path) {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(any(not(unix), target_vendor = "apple"))]
    let _ = dir;
}

#[cfg(test)]
mod tests;
