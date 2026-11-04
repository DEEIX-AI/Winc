//! 主程序这边：启动辅助进程，在后台线程里收发消息。
//!
//! 读和写各一个线程：主线程发消息只编码进待写的缓冲，辅助进程无响应时也不阻塞界面线程；收到的
//! 消息放进队列，并调用一次 `wake` 唤醒事件循环，主程序取完以后再收到消息才再次唤醒。辅助进程
//! 退出、读到结束时，读线程回收它。
//!
//! 收发各用一个带锁的队列而不用 `std::sync::mpsc`：通道每种元素类型都要带上一整套实现，本模块的
//! 用法简单，自己实现的代码更少。

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::protocol::{Decoder, FromHelper, Message, ToHelper};

/// 待写给辅助进程的字节。
#[derive(Default)]
struct Outbox {
    /// 待写的字节。
    bytes: Vec<u8>,
    /// 连接已经关闭（写线程把剩下的写完随即退出），或者写入失败（辅助进程不在了）。
    closed: bool,
}

/// 收到的消息。
#[derive(Default)]
struct Inbox {
    /// 收到的消息，先进先出。
    messages: VecDeque<FromHelper>,
    /// 读线程结束了：辅助进程退出了，或者发来的数据不对。
    closed: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 和一个辅助进程的连接。
///
/// 丢弃连接时关闭辅助进程的 stdin，辅助进程挂断所有终端后退出。
pub struct Client {
    outbox: Arc<(Mutex<Outbox>, Condvar)>,
    inbox: Arc<Mutex<Inbox>>,
    woken: Arc<AtomicBool>,
}

/// [`Client::poll`] 的结果。
#[derive(Debug)]
pub enum Poll {
    /// 一条消息。
    Message(FromHelper),
    /// 暂时没有消息了。
    Empty,
    /// 辅助进程退出了，或者发来的数据不对，连接已经断开。
    Closed,
}

impl Client {
    /// 启动 `program`（辅助进程 `winc-pty`），收到消息时在读线程里调用 `wake`。
    ///
    /// # Errors
    ///
    /// 程序不存在、不能执行，或者创建读、写线程失败时返回对应的错误。
    ///
    /// # Panics
    ///
    /// 启动子进程后取不到 stdin 或 stdout 时 panic；两者都设成了管道，正常情况下不会发生。
    pub fn start(program: &Path, wake: impl Fn() + Send + 'static) -> io::Result<Client> {
        let mut command = Command::new(program);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Windows 上图形程序启动命令行程序时会弹出控制台窗口：`CREATE_NO_WINDOW`（0x0800_0000）
        // 让辅助进程在后台运行，终端中的程序用的是辅助进程建的伪终端。
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("stdin 是管道");
        let stdout = child.stdout.take().expect("stdout 是管道");

        let outbox = Arc::new((Mutex::new(Outbox::default()), Condvar::new()));
        let pending = Arc::clone(&outbox);
        thread::Builder::new()
            .name("winc-pty 写".into())
            .spawn(move || write_messages(stdin, &pending))?;

        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let woken = Arc::new(AtomicBool::new(false));
        let (received, flag) = (Arc::clone(&inbox), Arc::clone(&woken));
        thread::Builder::new()
            .name("winc-pty 读".into())
            .spawn(move || {
                read_messages(stdout, &received, &flag, &wake);
                // 辅助进程退出了（或者数据不对）：再唤醒一次，让主程序发现并回收辅助进程。
                lock(&received).closed = true;
                wake();
                let _ = child.wait();
            })?;

        Ok(Client {
            outbox,
            inbox,
            woken,
        })
    }

    /// 发一条消息。
    ///
    /// 连接已经断开时什么也不做；下一次 [`Client::poll`] 会返回 `Closed`。
    pub fn send(&self, message: &ToHelper) {
        let (outbox, ready) = &*self.outbox;
        let mut outbox = lock(outbox);
        if !outbox.closed {
            message.encode(&mut outbox.bytes);
            ready.notify_one();
        }
    }

    /// 取一条收到的消息。
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
                // 先清除标志再检查一次：清除之后才到的消息会再次唤醒，不会漏掉。
                self.woken.store(false, Ordering::Release);
                take()
            }
            other => other,
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // 写线程写完剩下的字节就退出，辅助进程的 stdin 随之关闭，挂断终端后退出，读线程回收它。
        let (outbox, ready) = &*self.outbox;
        lock(outbox).closed = true;
        ready.notify_one();
    }
}

/// 写线程：有字节就整批写出去。
///
/// 连接已经关闭，或者写不进去（辅助进程不在了）时返回。
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

/// 读线程：把辅助进程发来的字节拼成消息放进队列，每批唤醒一次事件循环。
///
/// 辅助进程退出、数据不对时返回。
fn read_messages(mut stdout: impl Read, inbox: &Mutex<Inbox>, woken: &AtomicBool, wake: &dyn Fn()) {
    let mut decoder = Decoder::default();
    // 一次读 64 KiB：辅助进程一轮发的消息通常远小于它，读的次数少。
    let mut buf = vec![0; 64 << 10];
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            // 被信号打断的不算辅助进程退出，再读。
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        decoder.push(&buf[..n]);
        // 一次读到的消息一起放进队列：辅助进程一轮发的（比如重排以后的块和帧）主程序一起取到，
        // 不会只取到一半就去画。
        let mut batch = Vec::new();
        let result = loop {
            match decoder.next_message::<FromHelper>() {
                Ok(Some(message)) => batch.push(message),
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            }
        };
        lock(inbox).messages.extend(batch);
        if let Err(error) = result {
            eprintln!("winc: {error}");
            return;
        }
        if !woken.swap(true, Ordering::AcqRel) {
            wake();
        }
    }
}

#[cfg(test)]
mod tests;
