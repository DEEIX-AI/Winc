//! LSP 协议传输层：通过 stdio 与语言服务器通信。
//!
//! 采用三线程模型：
//! - Reader 线程：从 stdout 读取消息
//! - Writer 线程：向 stdin 写入消息
//! - Main thread：处理消息和回调

use crate::jsonrpc::{Message, Notification, Request, RequestId};
use crate::{Error, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// 传输层消息类型。
pub enum TransportMessage {
    /// 发送请求
    Request {
        id: RequestId,
        method: String,
        params: Option<serde_json::Value>,
        callback: ResponseCallback,
    },
    /// 发送通知
    Notification {
        method: String,
        params: Option<serde_json::Value>,
    },
    /// 关闭连接
    Shutdown,
}

/// 响应回调函数。
pub type ResponseCallback = Box<dyn FnOnce(Result<serde_json::Value>) + Send>;

/// 传输层句柄。
pub struct Transport {
    /// 发送消息的通道
    tx: Sender<TransportMessage>,
    /// 接收服务器消息的通道
    rx: Receiver<ServerMessage>,
    /// Reader 线程句柄
    _reader_thread: thread::JoinHandle<()>,
    /// Writer 线程句柄
    _writer_thread: thread::JoinHandle<()>,
}

/// 服务器消息类型。
#[derive(Debug)]
pub enum ServerMessage {
    /// 服务器发来的通知
    Notification(Notification),
    /// 服务器发来的请求
    Request(Request),
    /// 传输错误
    Error(Error),
    /// 服务器关闭
    Closed,
}

impl Transport {
    /// 创建新的传输层实例。
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        let (tx_to_writer, rx_from_transport) = unbounded::<TransportMessage>();
        let (tx_from_reader, rx_to_client) = unbounded::<ServerMessage>();

        // 存储待处理的请求回调
        let pending_requests: Arc<Mutex<HashMap<RequestId, ResponseCallback>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // 启动 writer 线程
        let pending_for_writer = Arc::clone(&pending_requests);
        let writer_thread = thread::Builder::new()
            .name("winc-lsp-writer".into())
            .spawn(move || {
                Self::writer_loop(stdin, rx_from_transport, pending_for_writer);
            })
            .expect("Failed to spawn LSP writer thread");

        // 启动 reader 线程
        let pending_for_reader = Arc::clone(&pending_requests);
        let reader_thread = thread::Builder::new()
            .name("winc-lsp-reader".into())
            .spawn(move || {
                Self::reader_loop(stdout, tx_from_reader, pending_for_reader);
            })
            .expect("Failed to spawn LSP reader thread");

        Self {
            tx: tx_to_writer,
            rx: rx_to_client,
            _reader_thread: reader_thread,
            _writer_thread: writer_thread,
        }
    }

    /// 发送请求。
    pub fn request(
        &self,
        id: RequestId,
        method: String,
        params: Option<serde_json::Value>,
        callback: ResponseCallback,
    ) -> Result<()> {
        self.tx
            .send(TransportMessage::Request {
                id,
                method,
                params,
                callback,
            })
            .map_err(|_| Error::StreamClosed)?;
        Ok(())
    }

    /// 发送通知。
    pub fn notify(&self, method: String, params: Option<serde_json::Value>) -> Result<()> {
        self.tx
            .send(TransportMessage::Notification { method, params })
            .map_err(|_| Error::StreamClosed)?;
        Ok(())
    }

    /// 关闭连接。
    pub fn shutdown(&self) {
        let _ = self.tx.send(TransportMessage::Shutdown);
    }

    /// 尝试接收服务器消息（非阻塞）。
    pub fn try_recv(&self) -> Result<Option<ServerMessage>> {
        match self.rx.try_recv() {
            Ok(msg) => Ok(Some(msg)),
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(Error::StreamClosed),
        }
    }

    /// 接收服务器消息（阻塞，带超时）。
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ServerMessage> {
        self.rx
            .recv_timeout(timeout)
            .map_err(|_| Error::Timeout(RequestId::Number(0)))
    }

    /// Writer 线程主循环。
    fn writer_loop(
        stdin: ChildStdin,
        rx: Receiver<TransportMessage>,
        pending_requests: Arc<Mutex<HashMap<RequestId, ResponseCallback>>>,
    ) {
        let mut writer = BufWriter::new(stdin);

        while let Ok(msg) = rx.recv() {
            match msg {
                TransportMessage::Request {
                    id,
                    method,
                    params,
                    callback,
                } => {
                    // 保存回调
                    pending_requests.lock().insert(id.clone(), callback);

                    // 发送请求
                    let request = Request::new(id, method, params);
                    if let Err(e) = Self::write_message(&mut writer, &Message::Request(request)) {
                        tracing::error!("Failed to write request: {}", e);
                        break;
                    }
                }
                TransportMessage::Notification { method, params } => {
                    let notification = Notification::new(method, params);
                    if let Err(e) =
                        Self::write_message(&mut writer, &Message::Notification(notification))
                    {
                        tracing::error!("Failed to write notification: {}", e);
                        break;
                    }
                }
                TransportMessage::Shutdown => {
                    break;
                }
            }
        }

        tracing::debug!("LSP writer thread exiting");
    }

    /// Reader 线程主循环。
    fn reader_loop(
        stdout: ChildStdout,
        tx: Sender<ServerMessage>,
        pending_requests: Arc<Mutex<HashMap<RequestId, ResponseCallback>>>,
    ) {
        let mut reader = BufReader::new(stdout);

        loop {
            match Self::read_message(&mut reader) {
                Ok(Message::Response(response)) => {
                    // 查找对应的回调
                    let callback = pending_requests.lock().remove(&response.id);
                    if let Some(callback) = callback {
                        let result = if let Some(error) = response.error {
                            Err(Error::Other(anyhow::anyhow!(
                                "RPC error {}: {}",
                                error.code,
                                error.message
                            )))
                        } else {
                            Ok(response.result.unwrap_or(serde_json::Value::Null))
                        };
                        callback(result);
                    } else {
                        tracing::warn!("Received response for unknown request: {:?}", response.id);
                    }
                }
                Ok(Message::Notification(notification)) => {
                    if tx
                        .send(ServerMessage::Notification(notification))
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(Message::Request(request)) => {
                    if tx.send(ServerMessage::Request(request)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to read message: {}", e);
                    let _ = tx.send(ServerMessage::Error(e));
                    break;
                }
            }
        }

        let _ = tx.send(ServerMessage::Closed);
        tracing::debug!("LSP reader thread exiting");
    }

    /// 写入一条消息（Content-Length 协议）。
    fn write_message(writer: &mut BufWriter<ChildStdin>, message: &Message) -> Result<()> {
        let json = serde_json::to_string(message)?;
        write!(writer, "Content-Length: {}\r\n\r\n{}", json.len(), json)?;
        writer.flush()?;
        tracing::trace!("Sent: {}", json);
        Ok(())
    }

    /// 读取一条消息（Content-Length 协议）。
    fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Message> {
        // 读取 headers
        let mut content_length: Option<usize> = None;
        let mut line = String::new();

        loop {
            line.clear();
            reader.read_line(&mut line)?;

            if line.is_empty() {
                return Err(Error::StreamClosed);
            }

            if line == "\r\n" {
                // 空行，headers 结束
                break;
            }

            // 解析 Content-Length
            if let Some(length_str) = line.strip_prefix("Content-Length: ") {
                content_length = Some(
                    length_str
                        .trim()
                        .parse()
                        .map_err(|_| Error::Other(anyhow::anyhow!("Invalid Content-Length")))?,
                );
            }
        }

        let content_length = content_length
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Missing Content-Length header")))?;

        // 读取消息体
        let mut buffer = vec![0u8; content_length];
        reader.read_exact(&mut buffer)?;

        let json = String::from_utf8(buffer)
            .map_err(|_| Error::Other(anyhow::anyhow!("Invalid UTF-8 in message")))?;

        tracing::trace!("Received: {}", json);

        let message: Message = serde_json::from_str(&json)?;
        Ok(message)
    }
}
