//! 远端主机：到服务进程的连接，以及经连接执行的文件操作。
//!
//! 每个连接目标至多一个连接，登记在进程级的表中，[`fs`](crate::fs) 中的函数遇到虚拟路径时按
//! 目标取用。连接是一个子进程（通常是 `ssh`）的标准输入输出，对端是服务进程（Unix 上的 `serve`）。
//!
//! 请求是同步的：发出后等待应答。同一连接上可以同时有多个请求，各自等待自己的应答。会等待
//! 网络的调用不能发生在界面线程上，见 [`forbid_blocking`]。除文件操作外，还可以在远端运行
//! 命令，见 [`exec`]。

#[cfg(unix)]
mod server;
#[cfg(test)]
mod tests;
mod wire;

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
pub use server::serve;
use wire::{Frame, Reader, Writer, kind};

use crate::Target;
use crate::fs::{Kind, Metadata, SaveError};

/// 到服务进程的一个连接。
///
/// 多个线程可以同时发出请求，各自等待自己的应答：一个运行很久的命令不妨碍其他线程读取文件。
/// 丢弃时关闭子进程的标准输入并结束子进程；服务进程在输入结束时结束它启动的命令并退出。
pub struct Connection {
    link: Arc<Link>,
    /// 承载连接的子进程；连接建立在其他字节流上时（见 [`Connection::over`]）没有。
    child: Mutex<Option<Child>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

/// 连接中由发出请求的线程和读取应答的线程共用的部分。
struct Link {
    /// 写请求的一端；连接中断后为 `None`。
    output: Mutex<Option<BufWriter<Box<dyn Write + Send>>>>,
    waiting: Mutex<Waiting>,
}

/// 等待应答的请求。
#[derive(Default)]
struct Waiting {
    /// 下一个请求号。
    next: u32,
    /// 请求号对应的接收端：应答的各帧按到达的顺序送入。
    replies: HashMap<u32, mpsc::Sender<Frame>>,
    /// 连接是否已中断：此后的请求立即失败。
    closed: bool,
}

impl Link {
    /// 登记一个新请求并写出，返回请求号和接收其应答的一端。
    fn send(&self, kind: u8, payload: &[u8]) -> io::Result<(u32, mpsc::Receiver<Frame>)> {
        let (sender, receiver) = mpsc::channel();
        let id = {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            if waiting.closed {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let id = waiting.next;
            waiting.next = waiting.next.wrapping_add(1);
            waiting.replies.insert(id, sender);
            id
        };
        if let Err(error) = self.write(kind, id, payload) {
            self.forget(id);
            return Err(error);
        }
        Ok((id, receiver))
    }

    /// 写出一帧；连接已中断时返回 [`io::ErrorKind::BrokenPipe`]。
    fn write(&self, kind: u8, id: u32, payload: &[u8]) -> io::Result<()> {
        let mut output = self.output.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(output) = output.as_mut() else {
            return Err(io::ErrorKind::BrokenPipe.into());
        };
        wire::write_frame(output, kind, id, payload)?;
        output.flush()
    }

    /// 不再等待请求 `id` 的应答。
    fn forget(&self, id: u32) {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.replies.remove(&id);
    }

    /// 读取应答并分发给各请求，直到连接中断；此时所有等待中的请求都以连接中断结束。
    fn receive(&self, mut input: impl io::Read) {
        // 读到不合协议的帧、输入结束、读取出错：都无法继续使用该连接。
        while let Ok(Some(frame)) = wire::read_frame(&mut input) {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            // 输出的一段之后还有后续的帧，保留接收端；其余类型是最后一帧。
            let sender = if frame.kind == kind::CHUNK {
                waiting.replies.get(&frame.id).cloned()
            } else {
                waiting.replies.remove(&frame.id)
            };
            drop(waiting);
            // 没有对应的请求：该请求已被放弃，丢弃其应答。
            if let Some(sender) = sender {
                let _ = sender.send(frame);
            }
        }
        self.close();
    }

    /// 关闭写请求的一端，并让所有等待中的请求立即以连接中断结束；此后的请求立即失败。
    fn close(&self) {
        *self.output.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.closed = true;
        // 丢弃发送端，等待中的线程从接收端得到「已断开」。
        waiting.replies.clear();
    }
}

/// 把最后一帧换成结果：成功时是载荷，失败时是对端报告的错误。
fn finish(frame: Frame) -> io::Result<Vec<u8>> {
    match frame.kind {
        kind::OK => Ok(frame.payload),
        kind::ERROR => Err(wire::decode_error(&frame.payload)),
        _ => Err(wire::invalid("unknown reply")),
    }
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
        Self::establish(input, output, Some(child))
    }

    /// 在一对字节流上建立连接，并与对端交换协议版本：对端从 `output` 的另一端读取请求，把应答
    /// 写到 `input` 的另一端。
    ///
    /// 用于对端不是子进程的情形，例如测试中在同一进程内运行的服务端。
    ///
    /// # Errors
    ///
    /// 对端不是服务进程，或协议版本不一致时返回对应的 `io::Error`。
    pub fn over(
        input: impl io::Read + Send + 'static,
        output: impl Write + Send + 'static,
    ) -> io::Result<Self> {
        Self::establish(input, output, None)
    }

    fn establish(
        input: impl io::Read + Send + 'static,
        output: impl Write + Send + 'static,
        child: Option<Child>,
    ) -> io::Result<Self> {
        let link = Arc::new(Link {
            output: Mutex::new(Some(BufWriter::new(Box::new(output)))),
            waiting: Mutex::default(),
        });
        let reader = {
            let link = Arc::clone(&link);
            thread::spawn(move || link.receive(BufReader::new(input)))
        };
        let connection = Self {
            link,
            child: Mutex::new(child),
            reader: Some(reader),
        };
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
    /// 对端应答的错误原样返回；连接中断时返回 [`io::ErrorKind::BrokenPipe`]，此后该连接上的
    /// 请求都会失败。
    fn request(&self, kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
        let (_, replies) = self.link.send(kind, payload)?;
        match replies.recv() {
            Ok(frame) => finish(frame),
            Err(_) => Err(io::ErrorKind::BrokenPipe.into()),
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

    /// 读取远端文件 `path` 从 `offset` 起至多 `len` 字节（不超过 [`wire::MAX_READ`]），返回数据和
    /// 是否已到文件末尾。
    pub(crate) fn read_range(
        &self,
        path: &[u8],
        offset: u64,
        len: u32,
    ) -> io::Result<(Vec<u8>, bool)> {
        let request = Writer::default()
            .bytes(path)
            .u64(offset)
            .u32(len.min(wire::MAX_READ))
            .finish();
        let reply = self.request(kind::READ, &request)?;
        let mut reply = Reader::new(&reply);
        let data = reply.bytes()?.to_vec();
        let end = reply.u8()? != 0;
        reply.finish()?;
        Ok((data, end))
    }

    /// 读取远端文件 `path` 的全部内容，按 [`wire::MAX_READ`] 分块请求。
    pub(crate) fn read(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let mut content = Vec::new();
        loop {
            let (data, end) = self.read_range(path, content.len() as u64, wire::MAX_READ)?;
            // 未到末尾却没有数据：对端不守约定，继续请求不会结束。
            if data.is_empty() && !end {
                return Err(wire::invalid("empty read before the end"));
            }
            content.extend_from_slice(&data);
            if end {
                return Ok(content);
            }
        }
    }

    /// 读取远端符号链接 `path` 的目标，返回其原始字节。
    pub(crate) fn read_link(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let reply = self.request(kind::READ_LINK, &Writer::default().bytes(path).finish())?;
        let mut reply = Reader::new(&reply);
        let target = reply.bytes()?.to_vec();
        reply.finish()?;
        Ok(target)
    }

    /// 发出一个应答为空的请求。
    fn command(&self, kind: u8, payload: Vec<u8>) -> io::Result<()> {
        let reply = self.request(kind, &payload)?;
        Reader::new(&reply).finish()
    }

    /// 创建远端目录 `path`；`all` 为真时连同各级上级目录。
    pub(crate) fn create_dir(&self, path: &[u8], all: bool) -> io::Result<()> {
        let payload = Writer::default().bytes(path).flag(all).finish();
        self.command(kind::CREATE_DIR, payload)
    }

    /// 新建远端文件 `path` 并写入 `contents`，已有条目时不覆盖。
    pub(crate) fn create_new(&self, path: &[u8], contents: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(path).bytes(contents).finish();
        self.command(kind::CREATE_NEW, payload)
    }

    /// 把 `contents` 覆盖写入远端文件 `path`。
    pub(crate) fn write(&self, path: &[u8], contents: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(path).bytes(contents).finish();
        self.command(kind::WRITE, payload)
    }

    /// 把远端的 `from` 改名为 `to`。
    pub(crate) fn rename(&self, from: &[u8], to: &[u8]) -> io::Result<()> {
        self.command(
            kind::RENAME,
            Writer::default().bytes(from).bytes(to).finish(),
        )
    }

    /// 把远端文件 `from` 复制到 `to`，返回复制的字节数。
    pub(crate) fn copy(&self, from: &[u8], to: &[u8]) -> io::Result<u64> {
        let payload = Writer::default().bytes(from).bytes(to).finish();
        let reply = self.request(kind::COPY, &payload)?;
        let mut reply = Reader::new(&reply);
        let copied = reply.u64()?;
        reply.finish()?;
        Ok(copied)
    }

    /// 把远端的符号链接 `from` 复制为指向同一目标的链接 `to`。
    pub(crate) fn copy_symlink(&self, from: &[u8], to: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(from).bytes(to).finish();
        self.command(kind::COPY_SYMLINK, payload)
    }

    /// 删除远端文件 `path`。
    pub(crate) fn remove_file(&self, path: &[u8]) -> io::Result<()> {
        self.command(kind::REMOVE_FILE, Writer::default().bytes(path).finish())
    }

    /// 把 `write` 写出的内容保存到远端文件 `path`，返回写入后的文件信息。
    ///
    /// 内容边写边分段发出，对端收齐后按 [`fs::save`](crate::fs::save) 的流程写入。`write` 失败时
    /// 通知对端丢弃已收到的内容，目标文件不变。
    pub(crate) fn save(
        &self,
        path: &[u8],
        bom: bool,
        in_memory: bool,
        write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> Result<Metadata, SaveError> {
        let begin = Writer::default()
            .bytes(path)
            .flag(bom)
            .flag(in_memory)
            .finish();
        let (id, replies) = self.link.send(kind::SAVE_BEGIN, &begin)?;
        let mut upload = Upload {
            link: &self.link,
            id,
            pending: Vec::new(),
        };
        let written = write(&mut upload).and_then(|()| upload.flush());
        let end = Writer::default().flag(written.is_ok()).finish();
        self.link.write(kind::SAVE_END, id, &end)?;
        let reply = match replies.recv() {
            Ok(frame) => finish(frame),
            Err(_) => Err(io::ErrorKind::BrokenPipe.into()),
        };
        // 本机一侧的错误（例如内容无法按所选的编码写出）优先于对端对「放弃」的应答。
        written?;
        let reply = reply?;
        let mut reply = Reader::new(&reply);
        match reply.u8()? {
            kind::SAVED => {
                let metadata = wire::read_metadata(&mut reply)?;
                reply.finish()?;
                Ok(metadata)
            }
            kind::SAVE_READ_ONLY => Err(SaveError::ReadOnly),
            kind::SAVE_IDENTITY => Err(SaveError::IdentityWouldChange),
            kind::SAVE_TEMPORARY => Err(SaveError::TemporaryFile),
            _ => Err(wire::invalid("unknown save result").into()),
        }
    }

    /// 在远端运行 `exec` 描述的命令并等待它结束，返回退出码和全部输出。
    ///
    /// 等待期间每隔 [`CANCEL_POLL`] 调用 `cancel`：返回真时通知对端强制结束命令（连同它启动的
    /// 子进程），命令结束后返回 [`io::ErrorKind::Interrupted`]。
    fn exec(&self, exec: &Exec, cancel: &dyn Fn() -> bool) -> io::Result<Output> {
        let (id, replies) = self.link.send(kind::EXEC, &exec.encode())?;
        let mut output = Output::default();
        let mut asked = false;
        loop {
            let frame = match replies.recv_timeout(CANCEL_POLL) {
                Ok(frame) => frame,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !asked && cancel() {
                        asked = true;
                        self.link.write(kind::CANCEL, id, &[])?;
                    }
                    continue;
                }
            };
            if frame.kind != kind::CHUNK {
                let reply = finish(frame)?;
                let mut reply = Reader::new(&reply);
                let has_code = reply.u8()? != 0;
                let code = reply.u32()?.cast_signed();
                reply.finish()?;
                output.code = has_code.then_some(code);
                return Ok(output);
            }
            let mut chunk = Reader::new(&frame.payload);
            let stream = chunk.u8()?;
            let data = chunk.bytes()?;
            chunk.finish()?;
            match stream {
                kind::STDOUT => output.stdout.extend_from_slice(data),
                kind::STDERR => output.stderr.extend_from_slice(data),
                _ => return Err(wire::invalid("unknown output stream")),
            }
        }
    }
}

/// 关闭输入之后等待对端自行退出的最长时间（秒），超过后强制结束子进程。
///
/// 对端在输入结束时先结束它启动的命令再退出，自身最多等待 2 秒；这里多留 1 秒，正常情况下
/// 不会抢在它之前结束子进程，否则远端的命令无人结束。
const SHUTDOWN: Duration = Duration::from_secs(3);

impl Connection {
    /// 断开：关闭输入，等待对端结束它启动的命令并退出，最长 [`SHUTDOWN`]，之后强制结束子进程。
    ///
    /// 仍在等待应答的请求立即以 [`io::ErrorKind::BrokenPipe`] 结束，不等对端退出。
    fn shutdown(&self) {
        self.link.close();
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(child) = child.as_mut() else {
            return;
        };
        let deadline = Instant::now() + SHUTDOWN;
        while Instant::now() < deadline {
            // 已退出，或无法查询：不再等待。
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let _ = child.kill();
    }

    /// 在后台线程上断开，调用方不等待。
    fn shutdown_in_background(self: Arc<Self>) {
        thread::spawn(move || self.shutdown());
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shutdown();
        // 回收子进程，不留下僵尸进程。
        let child = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(child) = child {
            let _ = child.wait();
        }
        // 子进程结束后其输出关闭，读取应答的线程随之退出。
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// 保存时一段内容的大小（字节，256 KiB）。
///
/// 每段是一帧，发送期间占用连接的写入一端；取值使其他请求在两段之间最多等待一段的发送时间，
/// 同时不让 16 MiB 的文档拆成过多的帧（64 帧）。
const UPLOAD_CHUNK: usize = 256 << 10;

/// 保存的内容的写入端：攒够 [`UPLOAD_CHUNK`] 就作为一帧发出。
struct Upload<'a> {
    link: &'a Link,
    id: u32,
    pending: Vec<u8>,
}

impl Write for Upload<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(data);
        while self.pending.len() >= UPLOAD_CHUNK {
            let rest = self.pending.split_off(UPLOAD_CHUNK);
            self.link.write(kind::SAVE_DATA, self.id, &self.pending)?;
            self.pending = rest;
        }
        Ok(data.len())
    }

    /// 发出尚未攒够一段的内容。
    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            self.link.write(kind::SAVE_DATA, self.id, &self.pending)?;
            self.pending.clear();
        }
        Ok(())
    }
}

/// 同一个连接上的两个远端路径：连接、第一个路径、第二个路径。
pub(crate) type Pair = (Arc<Connection>, Vec<u8>, Vec<u8>);

/// 把两个路径解析到同一个连接上：都是本机路径时返回 `None`，由调用方在本机执行。
///
/// # Errors
///
/// 一个在本机、一个在远端，或分属两个连接时返回 [`io::ErrorKind::Unsupported`]：跨主机的改名和
/// 复制尚不支持。其余同 [`connection`]。
pub(crate) fn pair(from: &Path, to: &Path) -> io::Result<Option<Pair>> {
    match (crate::locate(from), crate::locate(to)) {
        (None, None) => Ok(None),
        (Some(from), Some(to)) if from.target == to.target => {
            Ok(Some((connection(&from.target)?, from.path, to.path)))
        }
        _ => Err(io::ErrorKind::Unsupported.into()),
    }
}

/// 等待命令结束时检查是否被取消的间隔（毫秒）。取消由用户的操作或超时触发，50 ms 以内的
/// 延迟不可察觉；间隔内没有新的输出时线程才被唤醒一次。
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// 要在远端运行的命令。
///
/// 各部分是远端主机上的原始字节。命令继承服务进程的环境变量，[`Exec::env`] 设置的在其上覆盖。
#[derive(Debug, Clone, Default)]
pub struct Exec {
    program: Vec<u8>,
    cwd: Vec<u8>,
    args: Vec<Vec<u8>>,
    envs: Vec<(Vec<u8>, Vec<u8>)>,
    stdin: Vec<u8>,
}

impl Exec {
    /// 创建运行程序 `program` 的命令；`program` 不含 `/` 时在远端的 `PATH` 中查找。
    pub fn new(program: impl AsRef<[u8]>) -> Self {
        Self {
            program: program.as_ref().to_vec(),
            ..Self::default()
        }
    }

    /// 追加一个参数。
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<[u8]>) -> Self {
        self.args.push(arg.as_ref().to_vec());
        self
    }

    /// 设置一个环境变量。
    #[must_use]
    pub fn env(mut self, name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        self.envs
            .push((name.as_ref().to_vec(), value.as_ref().to_vec()));
        self
    }

    /// 设置写入命令标准输入的内容；命令、参数和输入合计不能超过一帧（约 1 MiB）。
    #[must_use]
    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = stdin.into();
        self
    }

    /// 编码为 [`kind::EXEC`] 的载荷。
    fn encode(&self) -> Vec<u8> {
        let mut out = Writer::default()
            .bytes(&self.program)
            .bytes(&self.cwd)
            .u32(self.args.len() as u32);
        for arg in &self.args {
            out = out.bytes(arg);
        }
        out = out.u32(self.envs.len() as u32);
        for (name, value) in &self.envs {
            out = out.bytes(name).bytes(value);
        }
        out.bytes(&self.stdin).finish()
    }
}

/// 命令运行的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// 退出码；命令被信号结束时为 `None`。
    pub code: Option<i32>,
    /// 标准输出的全部内容。
    pub stdout: Vec<u8>,
    /// 标准错误的全部内容。
    pub stderr: Vec<u8>,
}

/// 在目录 `dir` 所在的主机上以 `dir` 为工作目录运行 `exec`，等待它结束。
///
/// 等待期间反复调用 `cancel`，返回真时强制结束命令；调用方用它实现取消和超时。
///
/// # Errors
///
/// `dir` 不是虚拟路径时返回 [`io::ErrorKind::InvalidInput`]：本机的命令由调用方直接启动。
/// 没有连接、当前线程不允许等待、连接中断时返回连接的错误（见 [`fs`](crate::fs) 的说明）；
/// 远端无法启动命令时返回对端报告的错误；被取消时返回 [`io::ErrorKind::Interrupted`]。
/// 命令以非零的退出码结束不是错误，见 [`Output::code`]。
pub fn exec(dir: &Path, exec: &Exec, cancel: &dyn Fn() -> bool) -> io::Result<Output> {
    let Some(at) = crate::locate(dir) else {
        return Err(io::ErrorKind::InvalidInput.into());
    };
    let mut exec = exec.clone();
    exec.cwd = at.path;
    connection(&at.target)?.exec(&exec, cancel)
}

/// 进程中已建立的连接，按连接目标登记。
static CONNECTIONS: Mutex<Option<HashMap<Target, Arc<Connection>>>> = Mutex::new(None);

/// 登记到 `target` 的连接，此后该目标的虚拟路径经它访问；已有的连接被替换并断开。
pub fn register(target: Target, connection: Connection) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let replaced = connections
        .get_or_insert_with(HashMap::new)
        .insert(target, Arc::new(connection));
    drop(connections);
    if let Some(replaced) = replaced {
        replaced.shutdown_in_background();
    }
}

/// 断开到 `target` 的连接；没有连接时没有效果。
///
/// 不等待断开完成。正在进行的请求随后以 [`io::ErrorKind::BrokenPipe`] 结束，远端仍在运行的
/// 命令被强制结束。
pub fn disconnect(target: &Target) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let removed = connections.as_mut().and_then(|all| all.remove(target));
    drop(connections);
    if let Some(removed) = removed {
        removed.shutdown_in_background();
    }
}

/// 是否已登记到 `target` 的连接；连接可能已经中断，此时请求会失败。
pub fn is_connected(target: &Target) -> bool {
    let connections = CONNECTIONS.lock().unwrap_or_else(PoisonError::into_inner);
    connections
        .as_ref()
        .is_some_and(|all| all.contains_key(target))
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
