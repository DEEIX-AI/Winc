//! 与语言服务器子进程的连接：读、写各一个线程，参照 `winc-pty` 的 `Client`。
//!
//! 主线程发报文只编码进待写缓冲（服务器无响应也不阻塞界面线程）；收到的报文进队列，读线程
//! 调用一次 `wake` 唤醒事件循环，主程序 [`Connection::poll`] 取完以后再收到报文才再次唤醒。
//! 读线程读到结尾（服务器退出）时标记断开、再唤醒一次让主程序发现，并回收子进程。
//!
//! 服务器主动发来的请求（`workspace/configuration`、`client/registerCapability` 等）在
//! 读线程里就地自动应答，不进队列：应用层只关心响应与通知（诊断推送、消息）。应答一律是
//! 空结果——`workspace/configuration` 按请求里的 items 数量回 null 数组，其余回 null。
//! 收发各用一个带锁的队列而不用 `std::sync::mpsc`：与 `winc-pty` 一致，代码更少。

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use winc_json::Json;

use crate::rpc::{self, Message};

/// 待写给服务器的字节。
#[derive(Default)]
struct Outbox {
    /// 待写的字节。
    bytes: Vec<u8>,
    /// 连接已经关闭（写线程写完剩余字节随即退出），或者写入失败（服务器不在了）。
    closed: bool,
}

/// 收到的报文。
#[derive(Default)]
struct Inbox {
    /// 收到的报文，先进先出。
    messages: VecDeque<Message>,
    /// 读线程结束了：服务器退出了，或者发来的数据一直不对。
    closed: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 和一个语言服务器子进程的连接。
///
/// 丢弃连接时关闭服务器的 stdin；服务器按协议应先应答 `shutdown` 再收到 `exit` 自行退出，
/// 没退的话由读线程在 stdin 关闭后读到结尾时回收。
pub struct Connection {
    outbox: Arc<(Mutex<Outbox>, Condvar)>,
    inbox: Arc<Mutex<Inbox>>,
    woken: Arc<AtomicBool>,
}

/// [`Connection::poll`] 的结果。
#[derive(Debug)]
pub enum Poll {
    /// 一条报文（响应或通知）。
    Message(Message),
    /// 暂时没有报文了。
    Empty,
    /// 服务器退出了，连接已经断开。
    Closed,
}

impl Connection {
    /// 启动 `program`（语言服务器可执行文件），收到报文时在读线程里调用 `wake`。
    ///
    /// 服务器的 stderr 丢弃：正常时安静，崩溃时的报文由进程退出状态体现。
    ///
    /// # Errors
    ///
    /// 程序不存在、不能执行，或者创建读、写线程失败时返回对应的错误。
    ///
    /// # Panics
    ///
    /// 启动子进程后取不到 stdin 或 stdout 时 panic；两者都设成了管道，正常情况下不会发生。
    pub fn start(program: &Path, wake: impl Fn() + Send + 'static) -> io::Result<Connection> {
        let mut command = Command::new(program);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Windows 上图形程序启动命令行程序时会弹出控制台窗口：`CREATE_NO_WINDOW`（0x0800_0000）
        // 让服务器在后台运行。
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("stdin 是管道");
        let stdout = child.stdout.take().expect("stdout 是管道");

        let outbox = Arc::new((Mutex::new(Outbox::default()), Condvar::new()));
        let pending = Arc::clone(&outbox);
        thread::Builder::new()
            .name("winc-lsp 写".into())
            .spawn(move || write_messages(stdin, &pending))?;

        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let woken = Arc::new(AtomicBool::new(false));
        let (received, flag) = (Arc::clone(&inbox), Arc::clone(&woken));
        let replies = Arc::clone(&outbox);
        thread::Builder::new()
            .name("winc-lsp 读".into())
            .spawn(move || {
                read_messages(stdout, &received, &replies, &flag, &wake);
                // 服务器退出了：再唤醒一次，让主程序发现并回收它。
                lock(&received).closed = true;
                wake();
                let _ = child.wait();
            })?;

        Ok(Connection { outbox, inbox, woken })
    }

    /// 发一条报文。
    ///
    /// 连接已经断开时什么也不做；下一次 [`Connection::poll`] 会返回 `Closed`。
    pub fn send(&self, message: &Message) {
        queue_frame(&self.outbox, message);
    }

    /// 取一条收到的报文。
    ///
    /// 主程序被叫醒后应一直取到 `Empty` 或 `Closed`。
    pub fn poll(&self) -> Poll {
        let take = || {
            let mut inbox = lock(&self.inbox);
            match inbox.messages.pop_front() {
                Some(message) => Poll::Message(message),
                None if inbox.closed => Poll::Closed,
                None => Poll::Empty,
            }
        };
        match take() {
            Poll::Empty => {
                // 先清除标志再检查一次：清除之后才到的报文会再次唤醒，不会漏掉。
                self.woken.store(false, Ordering::Release);
                take()
            }
            other => other,
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // 写线程写完剩下的字节就退出，服务器的 stdin 随之关闭；先应答过 shutdown 的服务器
        // 自行退出，其余由读线程读到结尾时回收。
        let (outbox, ready) = &*self.outbox;
        lock(outbox).closed = true;
        ready.notify_one();
    }
}

/// 把一条报文编码进待写缓冲并唤醒写线程；连接关闭时丢弃。
fn queue_frame(outbox: &(Mutex<Outbox>, Condvar), message: &Message) {
    let (outbox, ready) = outbox;
    let mut pending = lock(outbox);
    if !pending.closed {
        pending.bytes.extend(rpc::frame(message));
        ready.notify_one();
    }
}

/// 写线程：有字节就整批写出去。
///
/// 连接已经关闭，或者写不进去（服务器不在了）时返回。
fn write_messages(mut stdin: impl Write, outbox: &(Mutex<Outbox>, Condvar)) {
    let (outbox, ready) = outbox;
    loop {
        let bytes = {
            let mut pending = lock(outbox);
            while pending.bytes.is_empty() && !pending.closed {
                pending = ready.wait(pending).unwrap_or_else(PoisonError::into_inner);
            }
            if pending.bytes.is_empty() {
                return;
            }
            std::mem::take(&mut pending.bytes)
        };
        if stdin
            .write_all(&bytes)
            .and_then(|()| stdin.flush())
            .is_err()
        {
            lock(outbox).closed = true;
            return;
        }
    }
}

/// 读线程：把服务器发来的字节拼成报文。响应与通知进队列并唤醒事件循环；服务器主动发来的
/// 请求就地自动应答，不进队列。
///
/// 服务器退出、数据一直不对时返回。
fn read_messages(
    mut stdout: impl Read,
    inbox: &Mutex<Inbox>,
    outbox: &(Mutex<Outbox>, Condvar),
    woken: &AtomicBool,
    wake: &dyn Fn(),
) {
    let mut decoder = rpc::Decoder::new();
    // 一次读 64 KiB：服务器一轮发的报文通常远小于它，读的次数少。
    let mut buf = vec![0; 64 << 10];
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            // 被信号打断的不算服务器退出，再读。
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        decoder.push(&buf[..n]);
        // 一次读到的报文一起放进队列：主程序一起取到，不会只取到一半就去画。
        let mut batch = Vec::new();
        while let Some(message) = decoder.next_message() {
            match message {
                Message::Request { id, method, params } => {
                    let result = auto_reply_result(&method, &params);
                    queue_frame(outbox, &Message::Response { id, result: Some(result), error: None });
                }
                other => batch.push(other),
            }
        }
        lock(inbox).messages.extend(batch);
        if !woken.swap(true, Ordering::AcqRel) {
            wake();
        }
    }
}

/// 对服务器请求的自动应答结果：`workspace/configuration` 按 items 数量回 null 数组，
/// 其余（`client/registerCapability`、`window/workDoneProgress/create` 等）回 null。
fn auto_reply_result(method: &str, params: &Option<Json>) -> Json {
    if method == "workspace/configuration" {
        let items = params
            .as_ref()
            .and_then(Json::as_object)
            .and_then(|map| map.get("items"))
            .and_then(Json::as_array)
            .map(<[Json]>::len)
            .unwrap_or(0);
        return Json::Array(vec![Json::Null; items]);
    }
    Json::Null
}

#[cfg(test)]
mod tests;
