//! LSP 客户端：管理单个语言服务器的连接和生命周期。

use crate::jsonrpc::RequestId;
use crate::transport::{ResponseCallback, ServerMessage, Transport};
use crate::{Error, OffsetEncoding, Result};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

slotmap::new_key_type! {
    /// 语言服务器的唯一标识符。
    pub struct ClientId;
}

/// LSP 客户端。
pub struct Client {
    /// 客户端 ID
    id: ClientId,
    /// 服务器名称
    name: String,
    /// 服务器进程
    _process: Child,
    /// 传输层
    transport: Transport,
    /// 请求计数器
    request_counter: AtomicU64,
    /// 服务器能力（初始化后设置）
    capabilities: RwLock<Option<lsp_types::ServerCapabilities>>,
    /// 位置编码方式
    offset_encoding: RwLock<OffsetEncoding>,
    /// 工作区根目录
    root_path: PathBuf,
    /// 工作区 URI
    root_uri: lsp_types::Uri,
    /// 初始化完成标志
    initialized: AtomicBool,
    /// 关闭标志
    shutting_down: AtomicBool,
}

impl Client {
    /// 启动一个新的语言服务器。
    pub fn spawn(
        id: ClientId,
        name: String,
        command: &str,
        args: &[String],
        root_path: PathBuf,
    ) -> Result<Self> {
        tracing::info!("Starting language server: {} at {:?}", name, root_path);

        // 启动服务器进程
        let mut process = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                Error::Other(anyhow::anyhow!("Failed to spawn server {}: {}", name, e))
            })?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Failed to get stdin")))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Failed to get stdout")))?;

        // 创建传输层
        let transport = Transport::new(stdin, stdout);

        // 创建工作区 URI (lsp-types 0.97 Uri 需要通过 FromStr parse)
        let root_uri = if let Ok(abs_path) = root_path.canonicalize() {
            // Windows: file:///C:/path, Unix: file:///path
            let path_str = abs_path.to_string_lossy().replace('\\', "/");
            let uri_string = if cfg!(windows) && !path_str.starts_with('/') {
                format!("file:///{}", path_str)
            } else {
                format!("file://{}", path_str)
            };
            uri_string.parse::<lsp_types::Uri>()
                .expect("Failed to parse URI")
        } else {
            let path_str = root_path.to_string_lossy().replace('\\', "/");
            let uri_string = if cfg!(windows) && !path_str.starts_with('/') {
                format!("file:///{}", path_str)
            } else {
                format!("file://{}", path_str)
            };
            uri_string.parse::<lsp_types::Uri>()
                .expect("Failed to parse URI")
        };

        Ok(Self {
            id,
            name,
            _process: process,
            transport,
            request_counter: AtomicU64::new(0),
            capabilities: RwLock::new(None),
            offset_encoding: RwLock::new(OffsetEncoding::Utf16),
            root_path,
            root_uri,
            initialized: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
        })
    }

    /// 获取客户端 ID。
    pub fn id(&self) -> ClientId {
        self.id
    }

    /// 获取服务器名称。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 获取服务器能力。
    pub fn capabilities(&self) -> Option<lsp_types::ServerCapabilities> {
        self.capabilities.read().clone()
    }

    /// 获取位置编码方式。
    pub fn offset_encoding(&self) -> OffsetEncoding {
        *self.offset_encoding.read()
    }

    /// 检查是否已初始化。
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// 生成新的请求 ID。
    fn next_request_id(&self) -> RequestId {
        RequestId::Number(self.request_counter.fetch_add(1, Ordering::Relaxed))
    }

    /// 初始化服务器。
    pub fn initialize(&self) -> Result<lsp_types::InitializeResult> {
        if self.is_initialized() {
            return Err(Error::Other(anyhow::anyhow!("Already initialized")));
        }

        let params = lsp_types::InitializeParams {
            process_id: Some(std::process::id()),
            root_path: Some(self.root_path.to_string_lossy().to_string()),
            root_uri: Some(self.root_uri.clone()),
            initialization_options: None,
            capabilities: lsp_types::ClientCapabilities {
                text_document: Some(lsp_types::TextDocumentClientCapabilities {
                    synchronization: Some(lsp_types::TextDocumentSyncClientCapabilities {
                        dynamic_registration: Some(false),
                        will_save: Some(true),
                        will_save_wait_until: Some(false),
                        did_save: Some(true),
                    }),
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(false),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    hover: Some(lsp_types::HoverClientCapabilities {
                        content_format: Some(vec![lsp_types::MarkupKind::Markdown]),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                workspace: Some(lsp_types::WorkspaceClientCapabilities {
                    configuration: Some(true),
                    did_change_configuration: Some(lsp_types::DynamicRegistrationClientCapabilities {
                        dynamic_registration: Some(false),
                    }),
                    ..Default::default()
                }),
                general: Some(lsp_types::GeneralClientCapabilities {
                    position_encodings: Some(vec![
                        lsp_types::PositionEncodingKind::UTF8,
                        lsp_types::PositionEncodingKind::UTF16,
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            trace: Some(lsp_types::TraceValue::Off),
            workspace_folders: None,
            client_info: Some(lsp_types::ClientInfo {
                name: "Winc".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            locale: None,
            work_done_progress_params: lsp_types::WorkDoneProgressParams {
                work_done_token: None,
            },
        };

        // 发送 initialize 请求
        let result = self.request_blocking::<lsp_types::request::Initialize>(params)?;

        // 保存服务器能力和编码方式
        *self.capabilities.write() = Some(result.capabilities.clone());

        // 从服务器能力中获取位置编码方式
        if let Some(encoding) = &result.capabilities.position_encoding {
            if let Some(enc) = OffsetEncoding::from_lsp(encoding.as_str()) {
                *self.offset_encoding.write() = enc;
            }
        }

        // 发送 initialized 通知
        self.notify::<lsp_types::notification::Initialized>(
            lsp_types::InitializedParams {},
        )?;

        self.initialized.store(true, Ordering::Release);
        tracing::info!("Language server {} initialized", self.name);

        Ok(result)
    }

    /// 关闭服务器。
    pub fn shutdown(&self) -> Result<()> {
        if self.shutting_down.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        tracing::info!("Shutting down language server: {}", self.name);

        // 发送 shutdown 请求
        let _: Option<()> = self
            .request_blocking::<lsp_types::request::Shutdown>(())
            .ok();

        // 发送 exit 通知
        self.notify::<lsp_types::notification::Exit>(())?;

        // 关闭传输层
        self.transport.shutdown();

        Ok(())
    }

    /// 发送请求（阻塞，等待响应）。
    pub fn request_blocking<R>(&self, params: R::Params) -> Result<R::Result>
    where
        R: lsp_types::request::Request,
        R::Params: serde::Serialize,
        R::Result: serde::de::DeserializeOwned,
    {
        let id = self.next_request_id();
        let method = R::METHOD.to_string();
        let params = serde_json::to_value(params)?;

        // 创建一个通道来接收响应
        let (tx, rx) = crossbeam_channel::bounded(1);

        self.transport.request(
            id.clone(),
            method,
            Some(params),
            Box::new(move |result| {
                let _ = tx.send(result);
            }),
        )?;

        // 等待响应（60秒超时）
        let result = rx
            .recv_timeout(Duration::from_secs(60))
            .map_err(|_| Error::Timeout(id))??;

        Ok(serde_json::from_value(result)?)
    }

    /// 发送请求（异步，带回调）。
    pub fn request<R>(&self, params: R::Params, callback: ResponseCallback) -> Result<()>
    where
        R: lsp_types::request::Request,
        R::Params: serde::Serialize,
    {
        let id = self.next_request_id();
        let method = R::METHOD.to_string();
        let params = serde_json::to_value(params)?;

        self.transport.request(id, method, Some(params), callback)
    }

    /// 发送通知。
    pub fn notify<N>(&self, params: N::Params) -> Result<()>
    where
        N: lsp_types::notification::Notification,
        N::Params: serde::Serialize,
    {
        let method = N::METHOD.to_string();
        let params = serde_json::to_value(params)?;

        self.transport.notify(method, Some(params))
    }

    /// 尝试接收服务器消息。
    pub fn try_recv(&self) -> Result<Option<ServerMessage>> {
        self.transport.try_recv()
    }

    /// 接收服务器消息（带超时）。
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ServerMessage> {
        self.transport.recv_timeout(timeout)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !self.shutting_down.load(Ordering::Acquire) {
            let _ = self.shutdown();
        }
    }
}
