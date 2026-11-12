//! 远端主机：到服务进程的连接，以及经连接执行的文件操作。
//!
//! 每个连接目标至多一个连接，登记在进程级的表中，[`fs`](crate::fs) 中的函数遇到虚拟路径时按
//! 目标取用。连接是一个子进程（通常是 `ssh`）的标准输入输出，对端是服务进程（Unix 上的 `serve`）。
//!
//! 请求是同步的：发出后等待应答，同一连接上的请求逐个进行。会等待网络的调用不能发生在界面
//! 线程上，见 [`forbid_blocking`]。

#[cfg(unix)]
mod server;
#[cfg(test)]
mod tests;
mod wire;

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

#[cfg(unix)]
pub use server::serve;
use wire::{Reader, Writer, kind};

use crate::Target;
use crate::fs::{Kind, Metadata};

/// 到服务进程的一个连接。
///
/// 丢弃时关闭子进程的标准输入并结束子进程；服务进程在输入结束时自行退出。
pub struct Connection(Mutex<Link>);

/// 连接的两个方向和请求号的计数。
struct Link {
    child: Child,
    input: BufReader<ChildStdout>,
    output: Option<BufWriter<ChildStdin>>,
    next: u32,
}

impl Connection {
    /// 启动 `command` 作为连接，并与对端交换协议版本。
    ///
    /// `command` 的标准输入输出接到本进程，标准错误丢弃。
    ///
    /// # Errors
    ///
    /// 无法启动命令、对端不是服务进程，或协议版本不一致时返回对应的 `io::Error`。
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let (input, output) = (child.stdout.take(), child.stdin.take());
        let (Some(input), Some(output)) = (input, output) else {
            return Err(io::Error::other("connection has no standard streams"));
        };
        let connection = Self(Mutex::new(Link {
            child,
            input: BufReader::new(input),
            output: Some(BufWriter::new(output)),
            next: 0,
        }));
        let hello = Writer::default().u32(wire::VERSION).finish();
        let reply = connection.request(kind::HELLO, &hello)?;
        let mut reply = Reader::new(&reply);
        if reply.u32()? != wire::VERSION {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "protocol version mismatch",
            ));
        }
        reply.finish()?;
        Ok(connection)
    }

    /// 发出一个请求并等待应答，返回应答的载荷。
    ///
    /// # Errors
    ///
    /// 对端应答的错误原样返回；连接中断、应答不合协议时返回对应的 `io::Error`，此后该连接上的
    /// 请求都会失败。
    fn request(&self, kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
        let mut link = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = link.next;
        link.next = link.next.wrapping_add(1);
        let Some(output) = link.output.as_mut() else {
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        let sent = wire::write_frame(output, kind, id, payload).and_then(|()| output.flush());
        let frame = match sent.and_then(|()| wire::read_frame(&mut link.input)) {
            Ok(Some(frame)) if frame.id == id => frame,
            Ok(Some(_)) => {
                // 请求号不符说明双方已不同步，无法继续使用该连接。
                link.output = None;
                return Err(wire::invalid("reply does not match the request"));
            }
            Ok(None) => {
                link.output = None;
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            Err(error) => {
                link.output = None;
                return Err(error);
            }
        };
        match frame.kind {
            kind::OK => Ok(frame.payload),
            kind::ERROR => Err(wire::decode_error(&frame.payload)),
            _ => {
                link.output = None;
                Err(wire::invalid("unknown reply"))
            }
        }
    }

    /// 取远端路径 `path` 的文件信息；`follow` 为真时跟随符号链接。
    pub(crate) fn metadata(&self, path: &[u8], follow: bool) -> io::Result<Metadata> {
        let request = if follow {
            kind::METADATA
        } else {
            kind::SYMLINK_METADATA
        };
        let reply = self.request(request, &Writer::default().bytes(path).finish())?;
        let mut reply = Reader::new(&reply);
        let metadata = wire::read_metadata(&mut reply)?;
        reply.finish()?;
        Ok(metadata)
    }

    /// 列出远端目录 `path`，返回各项的名字（远端的原始字节）和种类。
    pub(crate) fn read_dir(&self, path: &[u8]) -> io::Result<Vec<(Vec<u8>, Kind)>> {
        let reply = self.request(kind::READ_DIR, &Writer::default().bytes(path).finish())?;
        let mut reply = Reader::new(&reply);
        let count = reply.u32()?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let kind = wire::read_kind(reply.u8()?)?;
            let name = reply.bytes()?;
            // 名字来自对端：含 `/`、为空或是 `.`、`..` 的名字拼进路径后会指向别处，不接受。
            if name.is_empty() || name.contains(&b'/') || name == b"." || name == b".." {
                return Err(wire::invalid("invalid entry name"));
            }
            entries.push((name.to_vec(), kind));
        }
        reply.finish()?;
        Ok(entries)
    }

    /// 读取远端文件 `path` 的全部内容，按 [`wire::MAX_READ`] 分块请求。
    pub(crate) fn read(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let mut content = Vec::new();
        loop {
            let request = Writer::default()
                .bytes(path)
                .u64(content.len() as u64)
                .u32(wire::MAX_READ)
                .finish();
            let reply = self.request(kind::READ, &request)?;
            let mut reply = Reader::new(&reply);
            let data = reply.bytes()?;
            let end = reply.u8()? != 0;
            reply.finish()?;
            // 未到末尾却没有数据：对端不守约定，继续请求不会结束。
            if data.is_empty() && !end {
                return Err(wire::invalid("empty read before the end"));
            }
            content.extend_from_slice(data);
            if end {
                return Ok(content);
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let link = self.0.get_mut().unwrap_or_else(PoisonError::into_inner);
        // 先关闭输入，对端据此退出；再结束并回收子进程，不留下僵尸进程。
        link.output = None;
        let _ = link.child.kill();
        let _ = link.child.wait();
    }
}

/// 进程中已建立的连接，按连接目标登记。
static CONNECTIONS: Mutex<Option<HashMap<Target, Arc<Connection>>>> = Mutex::new(None);

/// 登记到 `target` 的连接，此后该目标的虚拟路径经它访问；已有的连接被替换并关闭。
pub fn register(target: Target, connection: Connection) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let replaced = connections
        .get_or_insert_with(HashMap::new)
        .insert(target, Arc::new(connection));
    drop(connections);
    // 关闭连接要等待子进程结束，不在持有表的锁时进行。
    drop(replaced);
}

/// 断开到 `target` 的连接；没有连接时没有效果。正在进行的请求完成后连接才关闭。
pub fn disconnect(target: &Target) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let removed = connections.as_mut().and_then(|all| all.remove(target));
    drop(connections);
    drop(removed);
}

/// 返回到 `target` 的连接。
///
/// # Errors
///
/// 当前线程不允许等待（见 [`forbid_blocking`]）时返回 [`io::ErrorKind::WouldBlock`]；没有到
/// 该目标的连接时返回 [`io::ErrorKind::NotConnected`]。
pub(crate) fn connection(target: &Target) -> io::Result<Arc<Connection>> {
    if BLOCKING_FORBIDDEN.get() {
        return Err(io::ErrorKind::WouldBlock.into());
    }
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    (connections.as_ref())
        .and_then(|all| all.get(target))
        .cloned()
        .ok_or_else(|| io::ErrorKind::NotConnected.into())
}

thread_local! {
    /// 是否不允许当前线程等待远端的应答。
    static BLOCKING_FORBIDDEN: Cell<bool> = const { Cell::new(false) };
}

/// 不允许当前线程等待远端的应答：此后该线程上对虚拟路径的操作立即返回
/// [`io::ErrorKind::WouldBlock`]，不发出请求。
///
/// 界面线程在启动时调用。远端的应答可能在数秒之后才到达，界面线程等待期间无法处理输入和
/// 重绘；访问远端文件的操作必须放到后台线程。本机路径不受影响。
pub fn forbid_blocking() {
    BLOCKING_FORBIDDEN.set(true);
}

/// 返回经 `ssh` 连接 `target` 并在远端运行程序 `server` 的命令。
///
/// 目标和 `ssh` 的其余设置取自用户的 `~/.ssh/config`。`-T` 不分配终端，`RemoteCommand=none`
/// 使配置中为该主机设置的登录命令不替换这里的程序；`server` 作为一个整体交给远端的 shell，
/// 其中的字符不被解释。
pub fn ssh_command(target: &Target, server: &str) -> Command {
    let mut command = Command::new("ssh");
    command
        .args(["-T", "-o", "RemoteCommand=none"])
        .arg(target.as_str())
        .arg("--")
        .arg(quote(server));
    command
}

/// 把 `text` 写成远端 shell 中的一个词：包在单引号内，其中的单引号写成 `'\''`。
fn quote(text: &str) -> OsString {
    format!("'{}'", text.replace('\'', r"'\''")).into()
}
