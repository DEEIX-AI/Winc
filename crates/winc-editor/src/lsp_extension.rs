//! Editor 的 LSP 扩展方法。

use crate::{Editor, LspManager, LspServerConfig};
use std::path::PathBuf;
use std::sync::Arc;
use winc_lsp::Registry;

impl Editor {
    /// 启用 LSP 支持。
    pub fn enable_lsp(&mut self) {
        if self.lsp_registry.is_none() {
            let registry = Arc::new(Registry::new());
            self.lsp_manager = Some(LspManager::new(Arc::clone(&registry)));
            self.lsp_registry = Some(registry);
            tracing::info!("LSP support enabled");
        }
    }

    /// 禁用 LSP 支持。
    pub fn disable_lsp(&mut self) {
        if let Some(registry) = self.lsp_registry.take() {
            registry.shutdown_all();
        }
        self.lsp_manager = None;
        tracing::info!("LSP support disabled");
    }

    /// 检查 LSP 是否已启用。
    pub fn is_lsp_enabled(&self) -> bool {
        self.lsp_registry.is_some()
    }

    /// 为文档启动语言服务器（如果尚未启动）。
    pub fn start_lsp_for_document(&mut self, doc_index: usize) -> Option<()> {
        let registry = self.lsp_registry.as_ref()?;
        let manager = self.lsp_manager.as_ref()?;
        let doc = self.documents.get_mut(doc_index)?;

        // 如果已经有 LSP 状态，跳过
        if doc.lsp_state.is_some() {
            return Some(());
        }

        let path = doc.path.as_ref()?;
        let language_id = crate::detect_language_id(path)?;

        // 查找匹配的服务器配置
        let config = LspServerConfig::find_for_language(&language_id, &self.lsp_configs)?;

        // 获取或启动服务器
        let client = if let Some(client) = registry.get_by_name(&config.name) {
            client
        } else {
            // 启动新服务器
            let root_path = self.workspace.as_ref()
                .and_then(|ws| ws.root.parent())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    path.parent().unwrap_or(path).to_path_buf()
                });

            tracing::info!("Starting LSP server: {} for {}", config.name, language_id);
            
            let client = registry
                .start_server(
                    config.name.clone(),
                    &config.command,
                    &config.args,
                    root_path,
                )
                .ok()?;

            // 初始化服务器
            if let Err(e) = client.initialize() {
                tracing::error!("Failed to initialize LSP server: {}", e);
                return None;
            }

            client
        };

        // 创建 URI (使用 FromStr parse)
        let path_str = path.to_string_lossy().replace('\\', "/");
        let uri_string = if cfg!(windows) && !path_str.starts_with('/') {
            format!("file:///{}", path_str)
        } else {
            format!("file://{}", path_str)
        };
        let uri = uri_string.parse::<winc_lsp::lsp::Uri>().ok()?;

        // 发送 didOpen 通知
        let text = doc.text().rope().map(|r| r.to_string()).unwrap_or_default();
        if let Err(e) = manager.did_open(&client, uri.clone(), language_id, 1, text) {
            tracing::error!("Failed to send didOpen: {}", e);
            return None;
        }

        // 创建 LSP 状态
        doc.lsp_state = Some(crate::LspDocumentState::new(uri, client.id()));

        tracing::info!("LSP started for document: {:?}", path);
        Some(())
    }

    /// 关闭文档的 LSP 连接。
    pub fn close_lsp_for_document(&mut self, doc_index: usize) {
        if let Some(registry) = &self.lsp_registry {
            if let Some(manager) = &self.lsp_manager {
                if let Some(doc) = self.documents.get_mut(doc_index) {
                    if let Some(state) = doc.lsp_state.take() {
                        if let Some(client) = registry.get(state.client_id) {
                            let _ = manager.did_close(&client, state.uri);
                        }
                    }
                }
            }
        }
    }

    /// 同步文档变更到 LSP 服务器。
    pub fn sync_lsp_changes(&mut self, doc_index: usize) {
        let registry = match &self.lsp_registry {
            Some(r) => r,
            None => return,
        };

        let manager = match &self.lsp_manager {
            Some(m) => m,
            None => return,
        };

        let doc = match self.documents.get_mut(doc_index) {
            Some(d) => d,
            None => return,
        };

        // Ekstrak info yang diperlukan tanpa borrow mutable
        let (client_id, uri, should_sync) = match &doc.lsp_state {
            Some(s) => {
                if !manager.should_sync(s) {
                    return;
                }
                (s.client_id, s.uri.clone(), true)
            }
            None => return,
        };

        if !should_sync {
            return;
        }

        let client = match registry.get(client_id) {
            Some(c) => c,
            None => return,
        };

        // Dapatkan text (borrow immutable doc)
        let text = doc.text().rope().map(|r| r.to_string()).unwrap_or_default();
        
        // Sekarang bisa borrow mutable state
        let state = match &mut doc.lsp_state {
            Some(s) => s,
            None => return,
        };
        
        // 生成变更事件（全文同步）
        let changes = vec![winc_lsp::lsp::TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text,
        }];

        state.increment_version();

        // 发送 didChange 通知
        if let Err(e) = manager.did_change(&client, uri, state.version, changes) {
            tracing::error!("Failed to send didChange: {}", e);
            return;
        }

        state.clear_pending();
    }

    /// 标记文档有待处理的变更。
    pub fn mark_lsp_pending(&mut self, doc_index: usize) {
        if let Some(doc) = self.documents.get_mut(doc_index) {
            if let Some(state) = &mut doc.lsp_state {
                state.mark_pending();
            }
        }
    }

    /// 处理 LSP 服务器消息。
    pub fn poll_lsp_messages(&mut self) {
        let registry = match &self.lsp_registry {
            Some(r) => r,
            None => return,
        };

        for client in registry.iter_clients() {
            while let Ok(Some(msg)) = client.try_recv() {
                self.handle_lsp_message(client.id(), msg);
            }
        }
    }

    /// 处理单个 LSP 消息。
    fn handle_lsp_message(&mut self, client_id: winc_lsp::ClientId, msg: winc_lsp::ServerMessage) {
        use winc_lsp::ServerMessage;

        match msg {
            ServerMessage::Notification(notif) => {
                self.handle_lsp_notification(client_id, notif);
            }
            ServerMessage::Request(req) => {
                tracing::warn!("Received LSP request (not yet handled): {}", req.method);
            }
            ServerMessage::Error(err) => {
                tracing::error!("LSP error: {}", err);
            }
            ServerMessage::Closed => {
                tracing::info!("LSP server closed");
            }
        }
    }

    /// 处理 LSP 通知。
    fn handle_lsp_notification(&mut self, client_id: winc_lsp::ClientId, notif: winc_lsp::Notification) {
        use winc_lsp::lsp::notification::Notification;

        match notif.method.as_str() {
            winc_lsp::lsp::notification::PublishDiagnostics::METHOD => {
                if let Some(params) = notif.params {
                    if let Ok(params) = serde_json::from_value::<winc_lsp::lsp::PublishDiagnosticsParams>(params) {
                        self.handle_diagnostics(client_id, params);
                    }
                }
            }
            winc_lsp::lsp::notification::ShowMessage::METHOD => {
                if let Some(params) = notif.params {
                    if let Ok(params) = serde_json::from_value::<winc_lsp::lsp::ShowMessageParams>(params) {
                        tracing::info!("LSP message: {:?} - {}", params.typ, params.message);
                    }
                }
            }
            winc_lsp::lsp::notification::LogMessage::METHOD => {
                if let Some(params) = notif.params {
                    if let Ok(params) = serde_json::from_value::<winc_lsp::lsp::LogMessageParams>(params) {
                        tracing::debug!("LSP log: {:?} - {}", params.typ, params.message);
                    }
                }
            }
            _ => {
                tracing::debug!("Unhandled LSP notification: {}", notif.method);
            }
        }
    }

    /// 处理诊断信息。
    fn handle_diagnostics(&mut self, client_id: winc_lsp::ClientId, params: winc_lsp::lsp::PublishDiagnosticsParams) {
        // Konversi diagnostics dulu (copy rope untuk konversi posisi)
        let converted_diagnostics: Vec<_> = self.documents.iter()
            .find(|d| d.lsp_state.as_ref().map_or(false, |s| s.client_id == client_id && s.uri == params.uri))
            .and_then(|doc| doc.text().rope().map(|rope| {
                params.diagnostics.iter().filter_map(|diag| {
                    let severity = diag.severity?;
                    let start = winc_lsp::lsp_pos_to_pos(
                        rope,
                        diag.range.start,
                        winc_lsp::OffsetEncoding::Utf16,
                    ).unwrap_or(0);
                    
                    let end = winc_lsp::lsp_pos_to_pos(
                        rope,
                        diag.range.end,
                        winc_lsp::OffsetEncoding::Utf16,
                    ).unwrap_or(start);

                    Some(crate::LspDiagnostic {
                        range: (start, end),
                        severity: severity.into(),
                        message: diag.message.clone(),
                        source: diag.source.clone(),
                    })
                }).collect()
            }))
            .unwrap_or_default();

        // Update state dengan diagnostics yang sudah dikonversi
        for doc in &mut self.documents {
            if let Some(state) = &mut doc.lsp_state {
                if state.client_id == client_id && state.uri == params.uri {
                    state.diagnostics = converted_diagnostics;
                    tracing::debug!("Updated diagnostics for {:?}: {} items", params.uri, state.diagnostics.len());
                    break;
                }
            }
        }
    }
}
