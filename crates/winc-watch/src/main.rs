//! 运行在 WSL 发行版内的文件监视程序。
//!
//! Windows 通过文件共享访问发行版中的目录，变化通知不一定送达；本程序在发行版内用 inotify 递归监视
//! 目录，把变化报告给 winc。用法是 `winc-watch <目录>`。不写任何文件，标准输入结束（winc 退出）时
//! 随之退出。
//!
//! 消息以 NUL 结尾，第一个字节是类型，路径按原样的字节传输：
//!
//! ```text
//! 标准输出：  S            开始监视
//!            P<路径>       路径发生变化
//!            R            通知丢失或监视出错，需要重新扫描
//! 标准输入：  W<路径>       另外监视该目录
//!            U<路径>       不再监视该目录
//! ```

use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use notify::event::EventKind;
use notify::{RecursiveMode, Watcher as _};

fn main() -> ExitCode {
    let Some(root) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("winc-watch: 需要指定目录");
        return ExitCode::FAILURE;
    };
    match run(&root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("winc-watch: {error}");
            ExitCode::FAILURE
        }
    }
}

/// 监视 `root`，直到标准输入结束。
fn run(root: &Path) -> notify::Result<()> {
    let mut watcher = notify::recommended_watcher(|event| send(&encode(&event)))?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    send(b"S\0");

    let mut input = io::stdin().lock();
    let mut command = Vec::new();
    loop {
        command.clear();
        match input.read_until(0, &mut command) {
            Ok(0) | Err(_) => return Ok(()),
            Ok(_) => {}
        }
        let Some(([kind, path @ ..], _)) = command.split_last().map(|(_, c)| (c, ())) else {
            continue;
        };
        let path = path_from_bytes(path);
        // 附加目录是可选的：监视失败时打开的目录仍然有效。
        let _ = match kind {
            b'W' => watcher.watch(&path, RecursiveMode::Recursive),
            b'U' => watcher.unwatch(&path),
            _ => Ok(()),
        };
    }
}

/// 把一次通知写成要发送的消息；打开、读取文件不算变化，结果为空。
fn encode(event: &notify::Result<notify::Event>) -> Vec<u8> {
    match event {
        Ok(event) if event.need_rescan() => b"R\0".to_vec(),
        Ok(event) if matches!(event.kind, EventKind::Access(_)) => Vec::new(),
        Ok(event) => event
            .paths
            .iter()
            .flat_map(|path| {
                let mut message = vec![b'P'];
                message.extend_from_slice(&path_bytes(path));
                message.push(0);
                message
            })
            .collect(),
        Err(_) => b"R\0".to_vec(),
    }
}

/// 发送消息；winc 已退出、管道已关闭时结束进程。
fn send(message: &[u8]) {
    if message.is_empty() {
        return;
    }
    let mut output = io::stdout().lock();
    if output
        .write_all(message)
        .and_then(|()| output.flush())
        .is_err()
    {
        std::process::exit(0);
    }
}

/// 返回路径的字节；Linux 上文件名不一定是 UTF-8，按原样传输。
#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

/// 返回路径的字节；其他平台只用于开发时运行，按 UTF-8 有损转换。
#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// 把消息中的字节换成路径。
#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec()).into()
}

/// 把消息中的字节换成路径；其他平台只用于开发时运行，按 UTF-8 有损转换。
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    OsString::from(String::from_utf8_lossy(bytes).into_owned()).into()
}

#[cfg(test)]
mod tests;
