//! LSP 事件处理：在主事件循环中poll LSP消息并处理。

use crate::app::App;
use winc_editor::Editor;

impl App {
    /// 在事件循环中定期 poll LSP 消息。
    pub(crate) fn poll_lsp(&mut self) {
        self.editor.poll_lsp_messages();
    }

    /// 文档打开后启动 LSP。
    pub(crate) fn try_start_lsp_for_active(&mut self) {
        if let Some(index) = self.editor.active {
            self.editor.start_lsp_for_document(index);
        }
    }

    /// 文档编辑后标记需要同步。
    pub(crate) fn mark_lsp_pending_for_active(&mut self) {
        if let Some(index) = self.editor.active {
            self.editor.mark_lsp_pending(index);
        }
    }

    /// 同步待处理的 LSP 变更（debounced）。
    pub(crate) fn sync_lsp_for_active(&mut self) {
        if let Some(index) = self.editor.active {
            self.editor.sync_lsp_changes(index);
        }
    }

    /// 关闭文档时清理 LSP 状态。
    pub(crate) fn close_lsp_for_document(&mut self, doc_index: usize) {
        self.editor.close_lsp_for_document(doc_index);
    }
}

/// LSP 操作命令（在 Command 之外单独处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspAction {
    /// 启用 LSP
    Enable,
    /// 禁用 LSP
    Disable,
    /// 转到定义
    GotoDefinition,
    /// 显示 Hover 信息
    ShowHover,
    /// 触发补全
    TriggerCompletion,
    /// 文档符号列表
    DocumentSymbols,
    /// 格式化文档
    FormatDocument,
    /// 重启语言服务器
    RestartServer,
}

impl App {
    /// 执行 LSP 操作。
    pub fn execute_lsp_action(&mut self, action: LspAction) {
        match action {
            LspAction::Enable => {
                self.editor.enable_lsp();
                tracing::info!("LSP enabled");
            }
            LspAction::Disable => {
                self.editor.disable_lsp();
                tracing::info!("LSP disabled");
            }
            LspAction::GotoDefinition => {
                self.lsp_goto_definition();
            }
            LspAction::ShowHover => {
                self.lsp_show_hover();
            }
            LspAction::TriggerCompletion => {
                self.lsp_trigger_completion();
            }
            LspAction::DocumentSymbols => {
                self.lsp_document_symbols();
            }
            LspAction::FormatDocument => {
                self.lsp_format_document();
            }
            LspAction::RestartServer => {
                tracing::info!("LSP restart not yet implemented");
            }
        }
    }

    fn lsp_goto_definition(&mut self) {
        let doc = match self.editor.active() {
            Some(d) => d,
            None => return,
        };

        let state = match &doc.lsp_state {
            Some(s) => s,
            None => {
                tracing::warn!("LSP not started for this document");
                return;
            }
        };

        let registry = match &self.editor.lsp_registry {
            Some(r) => r,
            None => return,
        };

        let manager = match &self.editor.lsp_manager {
            Some(m) => m,
            None => return,
        };

        let client = match registry.get(state.client_id) {
            Some(c) => c,
            None => return,
        };

        // 获取光标位置
        let cursor_pos = doc.selection.head();
        let lsp_pos = match winc_lsp::pos_to_lsp_pos(
            &doc.content.text(),
            cursor_pos,
            client.offset_encoding(),
        ) {
            Ok(pos) => pos,
            Err(e) => {
                tracing::error!("Failed to convert position: {}", e);
                return;
            }
        };

        tracing::info!("Requesting goto definition at {:?}", lsp_pos);

        // 发送请求（异步回调暂时只打印日志）
        let _ = manager.goto_definition(
            &client,
            state.uri.clone(),
            lsp_pos,
            Box::new(|result| {
                match result {
                    Ok(Some(response)) => {
                        tracing::info!("Goto definition response: {:?}", response);
                        // TODO: 实际打开文件并跳转
                    }
                    Ok(None) => {
                        tracing::info!("No definition found");
                    }
                    Err(e) => {
                        tracing::error!("Goto definition error: {}", e);
                    }
                }
            }),
        );
    }

    fn lsp_show_hover(&mut self) {
        let doc = match self.editor.active() {
            Some(d) => d,
            None => return,
        };

        let state = match &doc.lsp_state {
            Some(s) => s,
            None => return,
        };

        let registry = match &self.editor.lsp_registry {
            Some(r) => r,
            None => return,
        };

        let manager = match &self.editor.lsp_manager {
            Some(m) => m,
            None => return,
        };

        let client = match registry.get(state.client_id) {
            Some(c) => c,
            None => return,
        };

        let cursor_pos = doc.selection.head();
        let lsp_pos = match winc_lsp::pos_to_lsp_pos(
            &doc.content.text(),
            cursor_pos,
            client.offset_encoding(),
        ) {
            Ok(pos) => pos,
            Err(e) => {
                tracing::error!("Failed to convert position: {}", e);
                return;
            }
        };

        let _ = manager.hover(
            &client,
            state.uri.clone(),
            lsp_pos,
            Box::new(|result| {
                match result {
                    Ok(Some(hover)) => {
                        tracing::info!("Hover: {:?}", hover.contents);
                        // TODO: 显示 hover tooltip
                    }
                    Ok(None) => {
                        tracing::debug!("No hover information");
                    }
                    Err(e) => {
                        tracing::error!("Hover error: {}", e);
                    }
                }
            }),
        );
    }

    fn lsp_trigger_completion(&mut self) {
        let doc = match self.editor.active() {
            Some(d) => d,
            None => return,
        };

        let state = match &doc.lsp_state {
            Some(s) => s,
            None => return,
        };

        let registry = match &self.editor.lsp_registry {
            Some(r) => r,
            None => return,
        };

        let manager = match &self.editor.lsp_manager {
            Some(m) => m,
            None => return,
        };

        let client = match registry.get(state.client_id) {
            Some(c) => c,
            None => return,
        };

        let cursor_pos = doc.selection.head();
        let lsp_pos = match winc_lsp::pos_to_lsp_pos(
            &doc.content.text(),
            cursor_pos,
            client.offset_encoding(),
        ) {
            Ok(pos) => pos,
            Err(e) => {
                tracing::error!("Failed to convert position: {}", e);
                return;
            }
        };

        let _ = manager.completion(
            &client,
            state.uri.clone(),
            lsp_pos,
            Box::new(|result| {
                match result {
                    Ok(Some(completions)) => {
                        tracing::info!("Completions: {:?}", completions);
                        // TODO: 显示 completion 列表
                    }
                    Ok(None) => {
                        tracing::debug!("No completions");
                    }
                    Err(e) => {
                        tracing::error!("Completion error: {}", e);
                    }
                }
            }),
        );
    }

    fn lsp_document_symbols(&mut self) {
        let doc = match self.editor.active() {
            Some(d) => d,
            None => return,
        };

        let state = match &doc.lsp_state {
            Some(s) => s,
            None => return,
        };

        let registry = match &self.editor.lsp_registry {
            Some(r) => r,
            None => return,
        };

        let manager = match &self.editor.lsp_manager {
            Some(m) => m,
            None => return,
        };

        let client = match registry.get(state.client_id) {
            Some(c) => c,
            None => return,
        };

        let _ = manager.document_symbols(
            &client,
            state.uri.clone(),
            Box::new(|result| {
                match result {
                    Ok(Some(symbols)) => {
                        tracing::info!("Document symbols: {:?}", symbols);
                        // TODO: 显示符号列表
                    }
                    Ok(None) => {
                        tracing::debug!("No symbols");
                    }
                    Err(e) => {
                        tracing::error!("Document symbols error: {}", e);
                    }
                }
            }),
        );
    }

    fn lsp_format_document(&mut self) {
        let doc = match self.editor.active() {
            Some(d) => d,
            None => return,
        };

        let state = match &doc.lsp_state {
            Some(s) => s,
            None => return,
        };

        let registry = match &self.editor.lsp_registry {
            Some(r) => r,
            None => return,
        };

        let manager = match &self.editor.lsp_manager {
            Some(m) => m,
            None => return,
        };

        let client = match registry.get(state.client_id) {
            Some(c) => c,
            None => return,
        };

        let options = winc_lsp::lsp::FormattingOptions {
            tab_size: self.editor.active_tab_width() as u32,
            insert_spaces: true, // TODO: detect from document
            ..Default::default()
        };

        let _ = manager.formatting(
            &client,
            state.uri.clone(),
            options,
            Box::new(|result| {
                match result {
                    Ok(Some(edits)) => {
                        tracing::info!("Format edits: {} changes", edits.len());
                        // TODO: 应用编辑
                    }
                    Ok(None) => {
                        tracing::debug!("No formatting changes");
                    }
                    Err(e) => {
                        tracing::error!("Formatting error: {}", e);
                    }
                }
            }),
        );
    }
}
