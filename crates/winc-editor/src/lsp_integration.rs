//! LSP 集成：文档生命周期管理和 LSP 功能。

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winc_lsp::{lsp, Client, ClientId, Registry, Result};

/// LSP 诊断信息。
#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub range: (usize, usize),
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
}

/// 诊断严重程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl From<lsp::DiagnosticSeverity> for DiagnosticSeverity {
    fn from(severity: lsp::DiagnosticSeverity) -> Self {
        match severity {
            lsp::DiagnosticSeverity::ERROR => Self::Error,
            lsp::DiagnosticSeverity::WARNING => Self::Warning,
            lsp::DiagnosticSeverity::INFORMATION => Self::Info,
            lsp::DiagnosticSeverity::HINT => Self::Hint,
            _ => Self::Info,
        }
    }
}

/// LSP 文档状态。
#[derive(Debug)]
pub struct LspDocumentState {
    /// 文档的 LSP 版本号
    pub version: i32,
    /// 文档的 URI
    pub uri: lsp::Uri,
    /// 关联的语言服务器
    pub client_id: ClientId,
    /// 当前诊断
    pub diagnostics: Vec<LspDiagnostic>,
    /// 上次同步时间
    pub last_sync: Instant,
    /// 是否有待处理的变更
    pub pending_changes: bool,
}

impl LspDocumentState {
    pub fn new(uri: lsp::Uri, client_id: ClientId) -> Self {
        Self {
            version: 0,
            uri,
            client_id,
            diagnostics: Vec::new(),
            last_sync: Instant::now(),
            pending_changes: false,
        }
    }

    /// 增加版本号。
    pub fn increment_version(&mut self) {
        self.version += 1;
    }

    /// 标记有待处理的变更。
    pub fn mark_pending(&mut self) {
        self.pending_changes = true;
    }

    /// 清除待处理标记。
    pub fn clear_pending(&mut self) {
        self.pending_changes = false;
        self.last_sync = Instant::now();
    }
}

/// LSP 文档管理器。
pub struct LspManager {
    registry: Arc<Registry>,
    /// Debounce 延迟（毫秒）
    debounce_ms: u64,
}

impl LspManager {
    /// 创建新的 LSP 管理器。
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            debounce_ms: 200, // 200ms debounce
        }
    }

    /// 设置 debounce 延迟。
    pub fn set_debounce(&mut self, ms: u64) {
        self.debounce_ms = ms;
    }

    /// 检查是否应该同步（debounce check）。
    pub fn should_sync(&self, state: &LspDocumentState) -> bool {
        state.pending_changes && state.last_sync.elapsed() >= Duration::from_millis(self.debounce_ms)
    }

    /// 打开文档。
    pub fn did_open(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        language_id: String,
        version: i32,
        text: String,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::DidOpenTextDocumentParams {
            text_document: lsp::TextDocumentItem {
                uri,
                language_id,
                version,
                text,
            },
        };

        client.notify::<lsp::notification::DidOpenTextDocument>(params)
    }

    /// 关闭文档。
    pub fn did_close(&self, client: &Arc<Client>, uri: lsp::Uri) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::DidCloseTextDocumentParams {
            text_document: lsp::TextDocumentIdentifier { uri },
        };

        client.notify::<lsp::notification::DidCloseTextDocument>(params)
    }

    /// 文档变更。
    pub fn did_change(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        version: i32,
        changes: Vec<lsp::TextDocumentContentChangeEvent>,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::DidChangeTextDocumentParams {
            text_document: lsp::VersionedTextDocumentIdentifier {
                uri,
                version,
            },
            content_changes: changes,
        };

        client.notify::<lsp::notification::DidChangeTextDocument>(params)
    }

    /// 保存文档。
    pub fn did_save(&self, client: &Arc<Client>, uri: lsp::Uri, text: Option<String>) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::DidSaveTextDocumentParams {
            text_document: lsp::TextDocumentIdentifier { uri },
            text,
        };

        client.notify::<lsp::notification::DidSaveTextDocument>(params)
    }

    /// 转到定义。
    pub fn goto_definition(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        position: lsp::Position,
        callback: Box<dyn FnOnce(Result<Option<lsp::GotoDefinitionResponse>>) + Send>,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::GotoDefinitionParams {
            text_document_position_params: lsp::TextDocumentPositionParams {
                text_document: lsp::TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: lsp::WorkDoneProgressParams::default(),
            partial_result_params: lsp::PartialResultParams::default(),
        };

        client.request::<lsp::request::GotoDefinition>(
            params,
            Box::new(move |result| {
                let response = result.and_then(|value| {
                    serde_json::from_value(value).map_err(|e| e.into())
                });
                callback(response);
            }),
        )
    }

    /// Hover 信息。
    pub fn hover(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        position: lsp::Position,
        callback: Box<dyn FnOnce(Result<Option<lsp::Hover>>) + Send>,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::HoverParams {
            text_document_position_params: lsp::TextDocumentPositionParams {
                text_document: lsp::TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: lsp::WorkDoneProgressParams::default(),
        };

        client.request::<lsp::request::HoverRequest>(
            params,
            Box::new(move |result| {
                let response = result.and_then(|value| {
                    serde_json::from_value(value).map_err(|e| e.into())
                });
                callback(response);
            }),
        )
    }

    /// 代码补全。
    pub fn completion(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        position: lsp::Position,
        callback: Box<dyn FnOnce(Result<Option<lsp::CompletionResponse>>) + Send>,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::CompletionParams {
            text_document_position: lsp::TextDocumentPositionParams {
                text_document: lsp::TextDocumentIdentifier { uri },
                position,
            },
            work_done_progress_params: lsp::WorkDoneProgressParams::default(),
            partial_result_params: lsp::PartialResultParams::default(),
            context: None,
        };

        client.request::<lsp::request::Completion>(
            params,
            Box::new(move |result| {
                let response = result.and_then(|value| {
                    serde_json::from_value(value).map_err(|e| e.into())
                });
                callback(response);
            }),
        )
    }

    /// 文档符号。
    pub fn document_symbols(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        callback: Box<dyn FnOnce(Result<Option<lsp::DocumentSymbolResponse>>) + Send>,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::DocumentSymbolParams {
            text_document: lsp::TextDocumentIdentifier { uri },
            work_done_progress_params: lsp::WorkDoneProgressParams::default(),
            partial_result_params: lsp::PartialResultParams::default(),
        };

        client.request::<lsp::request::DocumentSymbolRequest>(
            params,
            Box::new(move |result| {
                let response = result.and_then(|value| {
                    serde_json::from_value(value).map_err(|e| e.into())
                });
                callback(response);
            }),
        )
    }

    /// 格式化文档。
    pub fn formatting(
        &self,
        client: &Arc<Client>,
        uri: lsp::Uri,
        options: lsp::FormattingOptions,
        callback: Box<dyn FnOnce(Result<Option<Vec<lsp::TextEdit>>>) + Send>,
    ) -> Result<()> {
        if !client.is_initialized() {
            return Ok(());
        }

        let params = lsp::DocumentFormattingParams {
            text_document: lsp::TextDocumentIdentifier { uri },
            options,
            work_done_progress_params: lsp::WorkDoneProgressParams::default(),
        };

        client.request::<lsp::request::Formatting>(
            params,
            Box::new(move |result| {
                let response = result.and_then(|value| {
                    serde_json::from_value(value).map_err(|e| e.into())
                });
                callback(response);
            }),
        )
    }
}

/// 从文件路径检测语言 ID。
pub fn detect_language_id(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| match ext {
            "rs" => Some("rust"),
            "py" => Some("python"),
            "js" => Some("javascript"),
            "ts" => Some("typescript"),
            "jsx" => Some("javascriptreact"),
            "tsx" => Some("typescriptreact"),
            "go" => Some("go"),
            "c" => Some("c"),
            "cpp" | "cc" | "cxx" => Some("cpp"),
            "h" | "hpp" => Some("cpp"),
            "java" => Some("java"),
            "cs" => Some("csharp"),
            "rb" => Some("ruby"),
            "php" => Some("php"),
            "swift" => Some("swift"),
            "kt" => Some("kotlin"),
            "lua" => Some("lua"),
            "sh" | "bash" => Some("shellscript"),
            "json" => Some("json"),
            "yaml" | "yml" => Some("yaml"),
            "toml" => Some("toml"),
            "xml" => Some("xml"),
            "html" => Some("html"),
            "css" => Some("css"),
            "scss" => Some("scss"),
            "md" => Some("markdown"),
            _ => None,
        })
        .map(String::from)
}

/// LSP 服务器配置。
#[derive(Debug, Clone)]
pub struct LspServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub languages: Vec<String>,
}

impl LspServerConfig {
    pub fn rust_analyzer() -> Self {
        Self {
            name: "rust-analyzer".to_string(),
            command: "rust-analyzer".to_string(),
            args: vec![],
            languages: vec!["rust".to_string()],
        }
    }

    pub fn typescript_language_server() -> Self {
        Self {
            name: "typescript-language-server".to_string(),
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            languages: vec![
                "typescript".to_string(),
                "javascript".to_string(),
                "typescriptreact".to_string(),
                "javascriptreact".to_string(),
            ],
        }
    }

    pub fn pyright() -> Self {
        Self {
            name: "pyright".to_string(),
            command: "pyright-langserver".to_string(),
            args: vec!["--stdio".to_string()],
            languages: vec!["python".to_string()],
        }
    }

    pub fn clangd() -> Self {
        Self {
            name: "clangd".to_string(),
            command: "clangd".to_string(),
            args: vec![],
            languages: vec!["c".to_string(), "cpp".to_string()],
        }
    }

    pub fn gopls() -> Self {
        Self {
            name: "gopls".to_string(),
            command: "gopls".to_string(),
            args: vec![],
            languages: vec!["go".to_string()],
        }
    }

    /// 获取默认的服务器配置列表。
    pub fn default_configs() -> Vec<Self> {
        vec![
            Self::rust_analyzer(),
            Self::typescript_language_server(),
            Self::pyright(),
            Self::clangd(),
            Self::gopls(),
        ]
    }

    /// 根据语言 ID 查找匹配的服务器配置。
    pub fn find_for_language<'a>(language_id: &'a str, configs: &'a [Self]) -> Option<&'a Self> {
        configs.iter().find(|config| {
            config.languages.iter().any(|lang| lang == language_id)
        })
    }
}
