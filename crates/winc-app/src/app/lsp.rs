//! LSP 集成：语言服务器的查找、启动、生命周期、文档同步与报文收发。
//!
//! 这一层只做「什么时候启动、发什么、收到什么」的调度，协议与进程在 `winc-lsp`：连接自带动静
//! 两个线程，读线程有报文或服务器退出时经 winit 的代理送 `UserEvent::Lsp` 回来，事件循环里
//! [`App::poll_lsp`] 取到空为止。启动在后台线程上完成（[`App::start_lsp`]），界面线程不等待。
//!
//! 生命周期遵循规格的三个约束：只在本地工作区打开了该语言的文档时才启动；全部文档关闭后空闲
//! 5 分钟退出；服务器异常退出提示一次，1 分钟内再次退出就不再自动重启（本会话不可用）。
//! 找不到可执行文件时在状态栏提示安装命令，本次会话不再尝试。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use winit::keyboard::{Key, NamedKey};
use winc_core::Lines;
use winc_editor::{Document, DocumentId};
use winc_i18n::t;
use winc_lsp::connection::{Connection, Poll};
use winc_lsp::message::{
    self, CompletionContext, CompletionParams, CompletionResponse, ContentChange, DidChangeParams,
    DidCloseParams, DidOpenParams, InitializeParams, PublishDiagnosticsParams, ServerCapabilities,
    ShowMessageParams, method,
};
use winc_lsp::position;
use winc_lsp::rpc::{Id, Message};
use winc_json::Json;

use super::*;

/// 全部文档关闭后到退出服务器进程的空闲时间。
pub(in crate::app) const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// 崩溃后不再自动重启的窗口：窗口内再次崩溃即视为崩溃循环。
pub(in crate::app) const CRASH_WINDOW: Duration = Duration::from_secs(60);
/// 补全列表一次最多显示的项数（与 Markdown 链接补全的可见行数相同）。
pub(crate) const COMPLETION_VISIBLE: usize = 10;

/// 已同步给服务器的文档：文档 URI 与同步时的版本号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Synced {
    /// 文档的 file URI（服务器的键）。
    pub uri: String,
    /// 已同步的文档版本（`revision`，作为 LSP 的 version 发）。
    pub revision: u64,
}

/// 本帧需要同步的一个文档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct SyncDoc {
    /// 文档标识。
    pub doc: DocumentId,
    /// 文档版本。
    pub revision: u64,
    /// 文档的 file URI。
    pub uri: String,
}

/// 一帧里对某个文档的同步动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum SyncAction {
    /// 关闭已不在列表里的文档。
    Close {
        /// 文档的 URI。
        uri: String,
    },
    /// 打开新出现的文档（全文）。
    Open(SyncDoc),
    /// 发送自 `previous` 以来的增量。
    Change {
        /// 文档与当前版本。
        doc: SyncDoc,
        /// 已同步的版本。
        previous: u64,
    },
}

/// 比较已同步的集合与当前打开的文档，返回本帧要发的动作。
///
/// 先关闭、再打开、最后发变更：关闭的文档不再占着服务器内存，新文档的全文先到，之后才是增量。
pub(in crate::app) fn sync_actions(
    synced: &HashMap<u64, Synced>,
    docs: &[SyncDoc],
) -> Vec<SyncAction> {
    let mut actions = Vec::new();
    for (id, state) in synced {
        if !docs.iter().any(|d| d.doc.0 == *id) {
            actions.push(SyncAction::Close { uri: state.uri.clone() });
        }
    }
    actions.extend(
        docs.iter()
            .filter(|doc| !synced.contains_key(&doc.doc.0))
            .cloned()
            .map(SyncAction::Open),
    );
    actions.extend(docs.iter().filter_map(|doc| {
        let state = synced.get(&doc.doc.0)?;
        (state.revision != doc.revision).then(|| SyncAction::Change {
            doc: doc.clone(),
            previous: state.revision,
        })
    }));
    actions
}

/// 服务器可执行文件名：Windows 带 `.exe`。
pub(in crate::app) fn server_file_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() }
}

/// 按 `dirs` 的顺序返回其中第一个含有 `name` 的路径；找不到时返回 `None`。
///
/// 只查本机：这些目录来自本机的 `PATH` 与 Go 的安装位置（远端工作区不启动服务器）。
#[allow(clippy::disallowed_methods, reason = "只访问本机：PATH 与 Go 的安装位置")]
pub(in crate::app) fn first_existing(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().map(|dir| dir.join(name)).find(|path| path.is_file())
}

/// 语言 `language` 要启动的程序。
///
/// 用户配置了命令时原样使用（不再查找，也不套默认表）；没有配置时按默认表在 `search`
/// （`PATH` 的各个目录，后接 Go 的标准安装位置）中查找，找不到返回 `None`（调用方提示安装）。
/// 语言没有服务器（或配置为空串禁用）时同样返回 `None`，调用方应先看
/// [`winc_config::Config::language_server`]。
pub(in crate::app) fn program_for(
    config: &winc_config::Config,
    language: &str,
    search: &[PathBuf],
) -> Option<PathBuf> {
    if config.language_servers.contains_key(language) {
        // 空串由 `language_server` 处理为禁用；这里非空即用户的命令。
        return config.language_server(language).map(PathBuf::from);
    }
    let command = config.language_server(language)?;
    let name = server_file_name(command);
    first_existing(search, &name)
}

/// 查找默认命令时的搜索目录：`PATH` 的各个目录，后接 Go 的标准安装位置。
pub(in crate::app) fn server_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    dirs.extend(go_bin_dirs());
    dirs
}

/// Go 的标准安装位置：`$GOPATH/bin`，以及 `~/go/bin`（Windows 是 `%USERPROFILE%\go\bin`）。
pub(in crate::app) fn go_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(gopath) = std::env::var_os("GOPATH") {
        // `GOPATH` 可以是多个目录，用平台的分隔符隔开。
        dirs.extend(std::env::split_paths(&gopath).map(|p| p.join("bin")));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(PathBuf::from(home).join("go").join("bin"));
    }
    dirs
}

/// 一条改动对应的 `contentChanges`：单条改动是增量，多条或不支持增量时全量。
pub(in crate::app) fn content_changes(
    doc: &Document,
    previous: u64,
    incremental: bool,
) -> Vec<ContentChange> {
    let full = || vec![ContentChange::Full(doc.text().substring(0, doc.text().end_pos()))];
    if !incremental {
        return full();
    }
    let Some(changes) = doc.changes_since(previous) else {
        return full();
    };
    // 一次改动多处（多光标）时一个 range 表达不了：全量。
    let [change] = &changes[..] else {
        return full();
    };
    // 起点之前的文本没有被这条改动影响，可以按当前文本换算；终点必须由被删除的文本推出
    // （删掉换行时，按当前文本的偏移算会落在别处）。
    let start = position_of(doc, change.start);
    vec![ContentChange::Incremental {
        range: winc_lsp::message::Range {
            start,
            end: advance(start, &change.removed),
        },
        text: change.inserted.clone(),
    }]
}

/// 从 `start` 起走过 `removed` 之后的 LSP 位置：换行符按行推进，其余按 UTF-16 列推进。
pub(in crate::app) fn advance(start: winc_lsp::message::Position, removed: &str) -> winc_lsp::message::Position {
    match removed.rfind('\n') {
        Some(at) => {
            let lines = removed.matches('\n').count() as u32;
            let last = &removed[at + 1..];
            winc_lsp::message::Position {
                line: start.line + lines,
                character: last.chars().map(|c| c.len_utf16() as u32).sum(),
            }
        }
        None => winc_lsp::message::Position {
            line: start.line,
            character: start.character
                + removed.chars().map(|c| c.len_utf16() as u32).sum::<u32>(),
        },
    }
}

/// 字符偏移对应的 LSP 位置。
pub(in crate::app) fn position_of(doc: &Document, pos: usize) -> winc_lsp::message::Position {
    let text = doc.text();
    let pos = pos.min(text.end_pos());
    let line = text.line_of(pos);
    let line_start = text.line_start(line);
    let content = text.line_content(line).slice().to_string();
    position::position_at(line as u32, &content, pos - line_start)
}

/// 诊断气泡的悬停延迟：鼠标移到诊断范围上停稳多久才弹消息，避免扫过时乱闪（取自调参：
/// 与 `VSCode` 的默认悬停延迟同档，够区分「扫过」与「查看」）。
pub(crate) const HOVER_STILL: Duration = Duration::from_millis(250);

/// 一个窗口的语言服务器状态。
///
/// 只挂在 [`App::lsp`] 一个字段上：连接、会话可用性、已同步的文档、空闲计时与崩溃限频。
pub(crate) struct LspState {
    /// 后台线程正在启动的连接；启动完成后由 [`LspState::adopt_start`] 取回。
    starting: Option<Arc<Mutex<Option<std::io::Result<Connection>>>>>,
    /// 与服务器的连接；`None` 是没启动、已退出或本会话不可用。
    connection: Option<Connection>,
    /// 本会话不可用的（启动命令，原因）：命令变了以后可以再试。
    unavailable: Option<(String, String)>,
    /// 启动命令与语言，不可用时据此判断配置是否已改。
    command: Option<String>,
    language: Option<String>,
    /// 工作区根目录的 URI，握手时发给服务器。
    root_uri: Option<String>,
    /// 已同步给服务器的文档，键是文档标识的数值（`DocumentId` 不实现 `Ord`）。
    synced: HashMap<u64, Synced>,
    /// 服务器是否已握手完成（`initialized` 已发出）。
    ready: bool,
    /// 服务器是否支持增量同步。
    incremental: bool,
    /// initialize 与 shutdown 请求的 id，用于对上响应。
    init_id: Option<u64>,
    shutdown_id: Option<u64>,
    /// 已发出 shutdown，正在等响应。
    closing: bool,
    /// 全部文档关闭后开始空闲计时的时刻；还有文档开着时为 `None`。
    idle_since: Option<Instant>,
    /// 上一次崩溃的时刻，用于判断崩溃循环。
    last_crash: Option<Instant>,
    /// 最近一次发请求用的 id。
    next_id: u64,
    /// 已经弹出过诊断气泡的位置与消息（文档、锚点、消息）：光标停在同一处时不重复弹出。
    diag_hint: Option<(DocumentId, usize, String)>,
    /// 诊断气泡的悬停待发：（鼠标位置，移到该位置的时刻）。位置变化才（重新）待发，
    /// 满 [`HOVER_STILL`] 时只评估一次后作废；按键也作废——悬停只由鼠标移动触发，
    /// 出现在静止鼠标下的新诊断不弹（同 `VSCode`）。
    hover_at: Option<((f32, f32), Instant)>,
    /// 上一帧的鼠标位置：与当前帧对比得出「鼠标移动了」。
    hover_pos: Option<(f32, f32)>,
}

impl Default for LspState {
    fn default() -> Self {
        LspState {
            starting: None,
            connection: None,
            unavailable: None,
            command: None,
            language: None,
            root_uri: None,
            synced: HashMap::new(),
            ready: false,
            incremental: false,
            init_id: None,
            shutdown_id: None,
            closing: false,
            idle_since: None,
            last_crash: None,
            next_id: 1,
            diag_hint: None,
            hover_at: None,
            hover_pos: None,
        }
    }
}

/// 后台启动的结果。
#[derive(Debug)]
pub(in crate::app) enum Adopted {
    /// 还没有结果（后台线程仍在启动）。
    Waiting,
    /// 启动成功，连接已装上，等待初始化。
    Started,
    /// 启动失败，本会话不可用；附原因。
    Failed(String),
}

impl LspState {
    /// 准备在后台启动 `program`：已在运行、正在启动或本会话不可用时返回 `None`。
    ///
    /// 返回的槽位由调用方交给后台线程，线程把连接结果放进去后用 `Lsp` 事件唤醒界面。
    pub(in crate::app) fn begin_start(
        &mut self,
        language: &str,
        command: &str,
        root_uri: Option<String>,
    ) -> Option<Arc<Mutex<Option<std::io::Result<Connection>>>>> {
        if self.connection.is_some() || self.starting.is_some() || self.unavailable.is_some() {
            return None;
        }
        self.language = Some(language.to_owned());
        self.command = Some(command.to_owned());
        self.root_uri = root_uri;
        self.synced.clear();
        self.ready = false;
        self.incremental = false;
        self.closing = false;
        self.init_id = None;
        self.shutdown_id = None;
        let slot = Arc::new(Mutex::new(None));
        self.starting = Some(Arc::clone(&slot));
        Some(slot)
    }

    /// 后台启动完成时取回结果：成功装上连接；失败记为不可用。
    pub(in crate::app) fn adopt_start(&mut self) -> Adopted {
        let Some(slot) = self.starting.clone() else {
            return Adopted::Waiting;
        };
        let result = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
        let Some(result) = result else {
            return Adopted::Waiting;
        };
        self.starting = None;
        match result {
            Ok(connection) => {
                self.connection = Some(connection);
                Adopted::Started
            }
            Err(error) => {
                let reason = error.to_string();
                let command = self.command.clone().unwrap_or_default();
                self.unavailable = Some((command, reason.clone()));
                Adopted::Failed(reason)
            }
        }
    }

    /// 轮询一次：连接存在时取一条报文，服务器退出时收回连接并返回 [`Poll::Closed`]；
    /// 没有连接时返回 [`Poll::Empty`]。
    pub(in crate::app) fn poll(&mut self) -> Poll {
        let Some(connection) = &self.connection else {
            return Poll::Empty;
        };
        match connection.poll() {
            Poll::Closed => {
                self.connection = None;
                self.synced.clear();
                self.ready = false;
                Poll::Closed
            }
            other => other,
        }
    }

    /// 发一条报文；连接不在时丢弃（调用方按降级处理，不重发）。
    pub(in crate::app) fn send(&self, message: &Message) {
        if let Some(connection) = &self.connection {
            connection.send(message);
        }
    }

    /// 服务器是否在运行。
    pub(in crate::app) fn running(&self) -> bool {
        self.connection.is_some()
    }

    /// 是否正在后台启动。
    pub(in crate::app) fn starting(&self) -> bool {
        self.starting.is_some()
    }

    /// 本会话是否不可用。
    pub(in crate::app) fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_ref().map(|(_, reason)| reason.as_str())
    }

    /// 记为不可用（找不到可执行文件等），本次会话不再尝试。
    pub(in crate::app) fn mark_unavailable(&mut self, language: &str, command: &str, reason: &str) {
        self.language = Some(language.to_owned());
        self.unavailable = Some((command.to_owned(), reason.to_owned()));
    }


    /// 配置里这条命令改过了：清掉不可用标记，下次需要启动时重新求值。
    pub(in crate::app) fn recheck_config(&mut self, config: &winc_config::Config) {
        let Some((command, _)) = &self.unavailable else {
            return;
        };
        let Some(language) = &self.language else {
            return;
        };
        let now = config.language_server(language);
        if now != Some(command.as_str()) {
            self.unavailable = None;
        }
    }

    /// 记下当前打开的、需要同步的文档数 `count`：第一次变成 0 时开始空闲计时，从 0 变回
    /// 非 0 时取消。
    pub(in crate::app) fn note_documents(&mut self, count: usize, now: Instant) {
        self.idle_since = match (self.idle_since, count) {
            (_, 0) => Some(self.idle_since.unwrap_or(now)),
            _ => None,
        };
    }

    /// 空闲到期的时刻（全部文档关闭之后）。
    pub(in crate::app) fn idle_deadline(&self) -> Option<Instant> {
        self.idle_since.map(|at| at + IDLE_TIMEOUT)
    }

    /// 服务器异常退出一次：返回是否允许自动重启。1 分钟窗口内的第二次崩溃起不再重启，
    /// 并把本会话标记为不可用。
    pub(in crate::app) fn crash(&mut self, now: Instant) -> bool {
        let looping = self
            .last_crash
            .is_some_and(|last| now.saturating_duration_since(last) < CRASH_WINDOW);
        self.last_crash = Some(now);
        if looping {
            let command = self.command.clone().unwrap_or_default();
            self.unavailable = Some((command, "服务器连续崩溃".to_owned()));
            return false;
        }
        true
    }

    /// 下一个请求 id。
    pub(in crate::app) fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// 发出的 initialize 请求 id。
    pub(in crate::app) fn take_init_id(&mut self) -> Option<u64> {
        self.init_id
    }

    /// 记下 initialize 请求的 id。
    pub(in crate::app) fn set_init_id(&mut self, id: u64) {
        self.init_id = Some(id);
    }

    /// 握手完成。
    pub(in crate::app) fn set_ready(&mut self, capabilities: ServerCapabilities) {
        self.ready = true;
        self.incremental = capabilities.incremental;
        self.init_id = None;
    }

    /// 是否已经握手完成。
    pub(in crate::app) fn ready(&self) -> bool {
        self.ready
    }

    /// 服务器是否支持增量同步。
    pub(in crate::app) fn incremental(&self) -> bool {
        self.incremental
    }

    /// 记下已同步的文档。
    pub(in crate::app) fn mark_synced(&mut self, doc: DocumentId, uri: String, revision: u64) {
        self.synced.insert(doc.0, Synced { uri, revision });
    }

    /// 已同步的文档。
    pub(in crate::app) fn synced(&self) -> &HashMap<u64, Synced> {
        &self.synced
    }

    /// 文档是否已同步给服务器。
    pub(in crate::app) fn is_synced(&self, doc: DocumentId) -> bool {
        self.synced.contains_key(&doc.0)
    }

    /// 去掉 URI 对应的已同步记录（文档关闭时）。
    pub(in crate::app) fn forget_uri(&mut self, uri: &str) {
        self.synced.retain(|_, state| state.uri != uri);
    }

    /// 工作区根目录的 URI。
    pub(in crate::app) fn root_uri(&self) -> Option<&str> {
        self.root_uri.as_deref()
    }

    /// 光标处的诊断气泡是否已经弹出过（同一范围、同样的消息）。
    pub(in crate::app) fn diag_hint_seen(&self, doc: DocumentId, anchor: usize, text: &str) -> bool {
        self.diag_hint
            .as_ref()
            .is_some_and(|(at, a, t)| *at == doc && *a == anchor && t == text)
    }

    /// 记下已经弹出过诊断气泡的位置与消息。
    pub(in crate::app) fn remember_diag_hint(&mut self, doc: DocumentId, anchor: usize, text: &str) {
        self.diag_hint = Some((doc, anchor, text.to_owned()));
    }

    /// 光标离开诊断范围：之后再次进入时重新弹出。
    pub(in crate::app) fn forget_diag_hint(&mut self) {
        self.diag_hint = None;
    }

    /// 按键后作废悬停待发：打字停顿后波浪线出现在静止的鼠标下也不会弹诊断气泡，
    /// 要等鼠标再次移动到诊断上。
    pub(in crate::app) fn disarm_diagnostic_hint(&mut self) {
        self.hover_at = None;
    }

    /// 悬停待发的到期时刻：事件循环据此安排唤醒，鼠标停下后不必再等其它事件才评估。
    pub(in crate::app) fn hover_deadline(&self) -> Option<Instant> {
        self.hover_at.map(|(_, since)| since + HOVER_STILL)
    }

    /// 开始退出：记下 shutdown 请求的 id。
    pub(in crate::app) fn begin_shutdown(&mut self, id: u64) {
        self.shutdown_id = Some(id);
        self.closing = true;
    }

    /// 是否已发出 shutdown。
    pub(in crate::app) fn closing(&self) -> bool {
        self.closing
    }

    /// 收到的响应是否是 shutdown 的。
    pub(in crate::app) fn take_shutdown_id(&mut self) -> Option<u64> {
        self.shutdown_id
    }

    /// 停止服务器：清掉连接与同步状态，本会话之后仍可重新启动。
    pub(in crate::app) fn stop(&mut self) {
        self.connection = None;
        self.starting = None;
        self.synced.clear();
        self.ready = false;
        self.incremental = false;
        self.idle_since = None;
        self.closing = false;
        self.init_id = None;
        self.shutdown_id = None;
    }
}

impl App {
    /// 启动语言服务器：创建连接在后台线程上完成，结果经 `Lsp` 事件取回（见
    /// [`LspState::adopt_start`]）。已在运行、正在启动或本会话不可用时什么也不做。
    pub(crate) fn start_lsp(&mut self, program: &Path, language: &str, command: &str) {
        let root_uri = self.editor.root().and_then(message::file_uri);
        let Some(slot) = self.lsp.begin_start(language, command, root_uri) else {
            return;
        };
        let program = program.to_path_buf();
        let proxy = self.link.proxy.clone();
        let wake = move || {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::Lsp);
            }
        };
        self.spawn_blocking_then(UserEvent::Lsp, move || {
            let result = Connection::start(&program, wake);
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        });
    }

    /// 取回后台启动的结果；失败时提示并记为不可用。
    fn adopt_lsp_start(&mut self) {
        match self.lsp.adopt_start() {
            Adopted::Started => self.request_redraw(),
            Adopted::Waiting => {}
            Adopted::Failed(error) => {
                self.flash(t!("editor-lsp-start-failed", error = error));
            }
        }
    }

    /// 语言服务器有报文（或退出了）：取到空为止，逐条交给 [`App::lsp_message`]。
    pub(crate) fn poll_lsp(&mut self) {
        self.adopt_lsp_start();
        loop {
            match self.lsp.poll() {
                Poll::Message(message) => self.lsp_message(message),
                Poll::Empty => return,
                Poll::Closed => {
                    self.lsp_closed();
                    return;
                }
            }
        }
    }

    /// 服务器退出：退出流程中的不再提示；异常退出提示一次，1 分钟内再次退出则不再重启。
    fn lsp_closed(&mut self) {
        if self.lsp.closing() {
            self.lsp.stop();
            return;
        }
        // 状态栏提示一次：不弹模态对话框，也不每帧重复。
        self.flash(t!("editor-lsp-crashed"));
        self.lsp.crash(Instant::now());
        self.completion.reset();
        self.diagnostics = Default::default();
        self.track_lsp_idle();
        self.request_redraw();
    }

    /// 处理服务器发来的一条报文。
    fn lsp_message(&mut self, message: Message) {
        match message {
            Message::Notification { method: name, params } => {
                let params = params.as_ref();
                match name.as_str() {
                    method::PUBLISH_DIAGNOSTICS => {
                        let Some(params) = params.and_then(PublishDiagnosticsParams::from_json)
                        else {
                            return;
                        };
                        self.lsp_diagnostics(params);
                    }
                    method::SHOW_MESSAGE => {
                        if let Some(params) = params.and_then(ShowMessageParams::from_json) {
                            // 服务器消息只记录：写进状态栏，不弹窗。
                            self.flash(params.message);
                        }
                    }
                    _ => {}
                }
            }
            Message::Response { id, result, error } => {
                if let Some(shutdown) = self.lsp.take_shutdown_id()
                    && id == Id::Number(shutdown as i64)
                {
                    self.send_lsp(&Message::Notification {
                        method: method::EXIT.into(),
                        params: None,
                    });
                    self.lsp.stop();
                    return;
                }
                if let Some(init) = self.lsp.take_init_id()
                    && id == Id::Number(init as i64)
                {
                    let capabilities = result
                        .as_ref()
                        .and_then(ServerCapabilities::from_json)
                        .unwrap_or(ServerCapabilities {
                            incremental: false,
                            completion_trigger_characters: Vec::new(),
                        });
                    self.lsp.set_ready(capabilities);
                    self.send_lsp(&Message::Notification {
                        method: method::INITIALIZED.into(),
                        params: None,
                    });
                    return;
                }
                let _ = error;
                let Some(result) = result else { return };
                if self.completion_response(&id, &result) {
                    self.request_redraw();
                }
            }
            Message::Request { .. } => {
                // 连接层已就地自动应答；进不了这里。
            }
        }
    }

    /// 补全响应：交给补全会话，过期或对不上的丢弃。
    pub(in crate::app) fn completion_response(&mut self, id: &Id, result: &Json) -> bool {
        let Id::Number(id) = id else {
            return false;
        };
        let Ok(id) = u64::try_from(*id) else {
            return false;
        };
        let response = CompletionResponse::from_json(result);
        let items = response.map(|r| r.items().to_vec()).unwrap_or_default();
        let current = self.editor.active().and_then(editor::completion::Completion::context);
        self.completion.response(id, current.as_ref(), items)
    }

    /// 诊断推送：整表替换该文档的诊断并重绘。
    pub(in crate::app) fn lsp_diagnostics(&mut self, params: PublishDiagnosticsParams) {
        let Some(path) = message::uri_to_path(&params.uri) else {
            return;
        };
        let Some(doc) = self
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path.as_path()))
        else {
            return;
        };
        let id = doc.id;
        let problems = editor::diagnostics::convert(doc, &params.diagnostics);
        self.diagnostics.replace(id, problems);
        self.request_redraw();
    }

    /// 给服务器发一条报文（连接不在时丢弃）。
    pub(crate) fn send_lsp(&mut self, message: &Message) {
        self.lsp.send(message);
    }

    /// 每帧：同步文档、启动或维持服务器。
    pub(in crate::app) fn track_lsp(&mut self) {
        // 没有事件循环（离屏测试、测量）时不启动：读线程的唤醒送不回来，进程也没有用处。
        if self.link.proxy.is_none() {
            return;
        }
        // 首帧呈现之前不做任何 LSP 工作（不探测、不启动进程），保证冷启动路径干净。
        if self.frame.presented == 0 {
            return;
        }
        self.adopt_lsp_start();
        self.lsp.recheck_config(&self.config);
        let now = Instant::now();
        // 本地工作区的根目录才支持；远端沿用「远端不跑本机工具」的边界。
        let local_root = self
            .editor
            .root()
            .is_some_and(|root| !winc_backend::is_remote(root));
        if !local_root {
            self.lsp.note_documents(0, now);
            self.sync_lsp_documents(Vec::new());
            return;
        }
        let docs = self.server_documents();
        if docs.is_empty() {
            self.lsp.note_documents(0, now);
            self.sync_lsp_documents(Vec::new());
            return;
        }
        self.lsp.note_documents(docs.len(), now);
        // 正在退出（空闲到时）：等它退出，不在关闭途中同步新的文档。
        if self.lsp.closing() {
            return;
        }
        // 首期一个窗口只跑一个服务器：以第一份文档的语言为准，其他语言的文档不启动也不
        // 同步（多服务器留给后续变更）。
        let language = docs[0].1.clone();
        // 还没启动：按配置找可执行文件，找不到就提示安装并放弃本次会话。
        if !self.lsp.running() && !self.lsp.starting() && self.lsp.unavailable().is_none() {
            let Some(command) = self.config.language_server(&language) else {
                return;
            };
            let command = command.to_owned();
            match program_for(&self.config, &language, &server_search_dirs()) {
                Some(program) => self.start_lsp(&program, &language, &command),
                None => {
                    self.lsp.mark_unavailable(&language, &command, "未找到可执行文件");
                    self.flash(t!("editor-lsp-not-installed", command = command));
                    return;
                }
            }
        }
        self.ensure_lsp_handshake();
        if !self.lsp.ready() {
            return;
        }
        let synced: Vec<SyncDoc> = docs
            .into_iter()
            .filter(|(_, doc_language)| *doc_language == language)
            .map(|(id, _)| {
                let doc = self.editor.document(id);
                SyncDoc {
                    doc: id,
                    revision: doc.map_or(0, |d| d.revision),
                    uri: doc
                        .and_then(|d| d.path.as_deref())
                        .and_then(message::file_uri)
                        .unwrap_or_default(),
                }
            })
            .filter(|d| !d.uri.is_empty())
            .collect();
        self.sync_lsp_documents(synced);
    }

    /// 首帧呈现后是否需要再画一帧来启动服务器：启动时打开着 Go 文档时不拖慢首帧。
    pub(in crate::app) fn lsp_start_after_first_frame(&self) -> bool {
        self.frame.presented == 1
            && self.link.proxy.is_some()
            && self.lsp.unavailable().is_none()
            && !self.lsp.running()
            && !self.lsp.starting()
            && self
                .editor
                .root()
                .is_some_and(|root| !winc_backend::is_remote(root))
            && !self.server_documents().is_empty()
    }

    /// 打开的、本机的、已读入的、有服务器配置的文本文档：（标识，语言）。
    fn server_documents(&self) -> Vec<(DocumentId, String)> {
        self.editor
            .documents
            .iter()
            .filter(|doc| doc.shows_text() && !doc.is_pending())
            .filter_map(|doc| {
                let path = doc.path.as_deref()?;
                // 远端目录与 proxy 连接上的文档不启动本机服务器（虚拟路径以主机名开头）。
                if winc_backend::is_remote(path) || doc.is_large() {
                    return None;
                }
                let language = doc.highlighter.language().id;
                self.config
                    .language_server(language)
                    .map(|_| (doc.id, language.to_owned()))
            })
            .collect()
    }

    /// 握手：连接在但还没发 initialize 时发一次。
    fn ensure_lsp_handshake(&mut self) {
        if !self.lsp.running() || self.lsp.ready() || self.lsp.take_init_id().is_some() {
            return;
        }
        let id = self.lsp.next_id();
        self.lsp.set_init_id(id);
        let params = InitializeParams {
            process_id: std::process::id(),
            root_uri: self.lsp.root_uri().map(str::to_owned),
        };
        self.send_lsp(&Message::Request {
            id: Id::Number(id as i64),
            method: method::INITIALIZE.into(),
            params: Some(params.to_json()),
        });
    }

    /// 按本帧的文档集合发 didOpen/didChange/didClose，并更新同步表与空闲计时。
    fn sync_lsp_documents(&mut self, docs: Vec<SyncDoc>) {
        // 关闭的文档不再显示诊断（服务器可能还会推一次，找不到文档就丢弃）。
        self.diagnostics.retain(|id| self.editor.document(id).is_some());
        for action in sync_actions(self.lsp.synced(), &docs) {
            match action {
                SyncAction::Close { uri } => {
                    self.lsp.forget_uri(&uri);
                    self.send_lsp(&Message::Notification {
                        method: method::DID_CLOSE.into(),
                        params: Some(DidCloseParams { uri }.to_json()),
                    });
                }
                SyncAction::Open(doc) => {
                    let Some(document) = self.editor.document(doc.doc) else {
                        continue;
                    };
                    let params = DidOpenParams {
                        uri: doc.uri.clone(),
                        language_id: document.highlighter.language().id.to_owned(),
                        version: doc.revision as i32,
                        text: document.text().substring(0, document.text().end_pos()),
                    };
                    self.lsp.mark_synced(doc.doc, doc.uri.clone(), doc.revision);
                    self.send_lsp(&Message::Notification {
                        method: method::DID_OPEN.into(),
                        params: Some(params.to_json()),
                    });
                }
                SyncAction::Change { doc, previous } => {
                    let Some(document) = self.editor.document(doc.doc) else {
                        continue;
                    };
                    let changes = content_changes(document, previous, self.lsp.incremental());
                    let params = DidChangeParams {
                        uri: doc.uri.clone(),
                        version: doc.revision as i32,
                        changes,
                    };
                    self.lsp.mark_synced(doc.doc, doc.uri.clone(), doc.revision);
                    self.send_lsp(&Message::Notification {
                        method: method::DID_CHANGE.into(),
                        params: Some(params.to_json()),
                    });
                }
            }
        }
        self.track_lsp_idle();
    }

    /// 全部文档关闭且空闲到时：发 shutdown 请求，收到响应后发 exit 并断开。
    fn track_lsp_idle(&mut self) {
        let now = Instant::now();
        if !self.lsp.running() || self.lsp.closing() {
            return;
        }
        if self
            .lsp
            .idle_deadline()
            .is_some_and(|deadline| now >= deadline)
        {
            let id = self.lsp.next_id();
            self.lsp.begin_shutdown(id);
            self.send_lsp(&Message::Request {
                id: Id::Number(id as i64),
                method: method::SHUTDOWN.into(),
                params: None,
            });
        }
    }

    /// 事件循环空闲时推进 LSP 的计时：补全请求的防抖、空闲退出。返回下一次唤醒的时刻。
    pub(in crate::app) fn lsp_tick(&mut self, now: Instant) -> Option<Instant> {
        let mut wake = self.completion.deadline();
        if let Some(context) = self.completion.tick(now) {
            self.send_completion(&context);
            wake = self.completion.deadline();
        }
        if let Some(deadline) = self.lsp.idle_deadline()
            && self.lsp.running()
            && !self.lsp.closing()
        {
            if now >= deadline {
                self.track_lsp_idle();
            } else {
                wake = Some(wake.map_or(deadline, |w| w.min(deadline)));
            }
        }
        wake
    }

    /// 发出补全请求；文档没同步给服务器时丢弃。
    fn send_completion(&mut self, context: &editor::completion::Context) {
        let Some(synced) = self.lsp.synced().get(&context.doc.0) else {
            return;
        };
        let Some(doc) = self.editor.document(context.doc) else {
            return;
        };
        let params = CompletionParams {
            uri: synced.uri.clone(),
            position: position_of(doc, context.caret),
            context: Some(CompletionContext {
                trigger_kind: if context.trigger.is_some() { 2 } else { 1 },
                trigger_character: context.trigger.clone(),
            }),
        };
        let id = self.lsp.next_id();
        self.completion.sent(id, context.clone());
        self.send_lsp(&Message::Request {
            id: Id::Number(id as i64),
            method: method::COMPLETION.into(),
            params: Some(params.to_json()),
        });
    }

    /// 补全列表开着时处理 ↑↓（换选中项）、Enter/Tab（接受）、Esc（关闭）；返回是否处理了
    /// 这个键。输入法组字、Vim 不在插入模式、焦点不在编辑区时不接管。
    pub(in crate::app) fn completion_key(&mut self, key: &Key) -> bool {
        if self.focus != Focus::Editor
            || !self.completion.shown()
            || !self.vim_typing()
            || !self.preedit_at(Focus::Editor).is_empty()
        {
            return false;
        }
        let m = self.pointer.modifiers;
        if m.shift_key() || m.control_key() || m.alt_key() || m.super_key() {
            return false;
        }
        let n = self.completion.rows().len();
        if n == 0 {
            return false;
        }
        match key {
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let down = matches!(key, Key::Named(NamedKey::ArrowDown));
                let selected = self.completion.selected();
                let step = if down { 1 } else { n - 1 };
                self.completion.select((selected + step) % n, COMPLETION_VISIBLE);
            }
            Key::Named(NamedKey::Enter | NamedKey::Tab) => self.accept_completion(),
            Key::Named(NamedKey::Escape) => self.completion.escaped(),
            _ => return false,
        }
        self.request_redraw();
        true
    }

    /// 鼠标点选补全列表的第 `row` 行：高亮它并接受。
    pub(in crate::app) fn click_completion(&mut self, row: usize) {
        self.completion.select(row, COMPLETION_VISIBLE);
        self.accept_completion();
    }

    /// 鼠标在补全列表上时按滚轮位移 `dy` 滚动列表，返回是否已处理。
    pub(in crate::app) fn scroll_completion(&mut self, (x, y): (f32, f32), dy: f32) -> bool {
        let over = self
            .hits
            .completion_card
            .is_some_and(|card| card.contains(x, y));
        if !over {
            return false;
        }
        let n = self.completion.rows().len();
        if n <= COMPLETION_VISIBLE {
            return true;
        }
        let Some(row) = self
            .hits
            .completion
            .first()
            .map(|(r, _)| r.h)
            .filter(|h| *h > 0.0)
        else {
            return true;
        };
        // 累积不足一行的位移：触控板的小步滚动也能平滑滚到下一行。
        let steps = (dy / row).trunc();
        let first = self.completion.first() as isize - steps as isize;
        self.completion
            .scroll_to(first.clamp(0, (n - COMPLETION_VISIBLE) as isize) as usize, COMPLETION_VISIBLE);
        self.request_redraw();
        true
    }

    /// 接受当前高亮的候选：按服务器的 `textEdit` 或已输入的词替换，整段插入是一个撤销单元。
    pub(in crate::app) fn accept_completion(&mut self) {
        let Some((item, context)) = self.completion.take_selected() else {
            return;
        };
        if self.editor.active().map(|d| d.id) != Some(context.doc) {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let edit = editor::completion::edit_for(doc, &context, &item);
        let lines_before = self.editor.active().map_or(0, |d| d.text().line_count());
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        editor::completion::apply_edit(doc, &edit);
        let current = self.editor.active().and_then(editor::completion::Completion::context);
        self.completion.accepted(current);
        let lines_after = self.editor.active().map_or(0, |d| d.text().line_count());
        if lines_after != lines_before {
            self.relayout();
        }
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 文档 `id` 的诊断按行分组，颜色取自主题：错误与警告两档。
    ///
    /// 只处理可见范围由绘制方决定；这里返回整个文档的分组（大文档的诊断本身不多）。
    pub(crate) fn diagnostic_lines(
        &self,
        id: DocumentId,
    ) -> Vec<(usize, Vec<editor::diagnostics::Underline>)> {
        let Some(doc) = self.editor.document(id) else {
            return Vec::new();
        };
        let p = self.palette;
        (self.diagnostics.by_line(doc).into_iter())
            .map(|(line, rows)| {
                let rows = rows
                    .into_iter()
                    .map(|(start, end, severity)| editor::diagnostics::Underline {
                        start,
                        end,
                        color: match severity {
                            winc_lsp::message::Severity::Error => p.error,
                            winc_lsp::message::Severity::Warning => p.warning,
                        },
                    })
                    .collect();
                (line, rows)
            })
            .collect()
    }

    /// 每帧：鼠标移动到诊断波浪线上停稳后弹出消息气泡，移开后淡出。
    ///
    /// 悬停只由**鼠标移动**触发（同 `VSCode`）：与上一帧位置不同才算一次移动，移到某处
    /// 才开始待发，满 [`HOVER_STILL`] 时只评估一次——当时鼠标下有诊断就弹，没有就作废。
    /// 因此打字时出现在静止鼠标下的新诊断不弹，打字停顿后也不弹；任意按键作废待发，
    /// 补全列表显示期间不弹。按严重级别从高到低合并同一处的多条诊断；鼠标停着不动时
    /// 保持气泡，不自动超时消失。
    pub(in crate::app) fn track_diagnostic_hint(&mut self, now: Instant) {
        let Some(doc) = self.editor.active() else {
            self.end_diagnostic_hint();
            return;
        };
        // 补全列表打开时不弹诊断气泡（VSCode 的建议列表显示时同样隐藏悬停）。
        if self.completion.shown() {
            self.end_diagnostic_hint();
            return;
        }
        // 与上一帧的鼠标位置不同才算一次「鼠标移动」：重新开始悬停计时。
        let (x, y) = self.pointer.mouse;
        let moved = self.lsp.hover_pos != Some((x, y));
        self.lsp.hover_pos = Some((x, y));
        if moved {
            self.lsp.hover_at = Some(((x, y), now));
        }
        if !doc.shows_text() || !self.pointer.cursor_inside || !self.over_editor_text() {
            self.end_diagnostic_hint();
            return;
        }
        let id = doc.id;
        let Some(pos) = self.pos_at(x, y) else {
            self.end_diagnostic_hint();
            return;
        };        // 先在只读借用里算出要显示的内容，之后才能改界面状态。
        let found = {
            let mut hits = self.diagnostics.at(id, pos);
            if hits.is_empty() {
                None
            } else {
                let anchor = hits.iter().map(|p| p.start).min().unwrap_or(0);
                hits.sort_by_key(|p| match p.severity {
                    winc_lsp::message::Severity::Error => 0,
                    winc_lsp::message::Severity::Warning => 1,
                });
                let text = hits
                    .iter()
                    .map(|p| p.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                // 排序后首条即最高级别（错误 0，警告 1）。
                let level = hits
                    .first()
                    .map_or(0, |p| match p.severity {
                        winc_lsp::message::Severity::Error => 0,
                        winc_lsp::message::Severity::Warning => 1,
                    });
                Some((anchor, text, level))
            }
        };
        let Some((anchor, text, level)) = found else {
            // 鼠标仍在文字上但此处没有诊断：旧气泡淡出，待发保留到到期时再看一次。
            self.fade_diagnostic_hint();
            return;
        };
        // 同一处同样的消息：鼠标停着就一直显示（待发随之作废，避免反复报到期的唤醒）。
        if self.diagnostics_hint_seen(id, anchor, &text) {
            self.lsp.disarm_diagnostic_hint();
            self.hold_diagnostic_hint();
            return;
        }
        // 唯一的弹出路径：鼠标移动过来并停满 HOVER_STILL。到点只看一次，没赶上就作废；
        // 其余情形（按键后、静止鼠标下的新诊断）都只淡出。
        match self.lsp.hover_at {
            Some((at, _)) if at != (x, y) => {
                self.lsp.disarm_diagnostic_hint();
                self.end_diagnostic_hint();
            }
            Some((_, since)) if now.duration_since(since) >= HOVER_STILL => {
                self.lsp.disarm_diagnostic_hint();
                self.lsp.remember_diag_hint(id, anchor, &text);
                self.hint_at_diagnostic(text, anchor, level);
            }
            // 还没停稳：保持待发，由 `hover_tick` 在到期时唤醒评估；旧气泡先淡出。
            Some(_) => self.fade_diagnostic_hint(),
            None => self.end_diagnostic_hint(),
        }
    }

    /// 悬停待发的到期检查：到点就地评估一次，不等下一帧（鼠标停下后没有新事件）。
    ///
    /// 返回下一次需要检查的时刻；鼠标不在编辑区文字上、按键后或评估完毕时为 `None`。
    pub(in crate::app) fn hover_tick(&mut self, now: Instant) -> Option<Instant> {
        let at = self.lsp.hover_deadline()?;
        if now < at {
            return Some(at);
        }
        self.track_diagnostic_hint(now);
        self.lsp.hover_deadline()
    }

    /// 鼠标不在诊断上：诊断气泡淡出、清掉弹出记忆，悬停待发一并作废。
    fn end_diagnostic_hint(&mut self) {
        self.lsp.disarm_diagnostic_hint();
        self.fade_diagnostic_hint();
    }

    /// 气泡淡出并清掉弹出记忆；不动悬停待发。
    fn fade_diagnostic_hint(&mut self) {
        self.lsp.forget_diag_hint();
        if self.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic) {
            self.dismiss_hint();
        }
    }

    /// 当前光标处的诊断气泡是否已经针对同一处、同样的消息弹出过。
    fn diagnostics_hint_seen(&self, doc: DocumentId, anchor: usize, text: &str) -> bool {
        self.lsp.diag_hint_seen(doc, anchor, text)
            || self.diagnostic_hint_is(text, anchor)
    }

    /// 每帧：按光标处的上下文推进补全会话。输入法组字期间不看不改（按键交给输入法）。
    pub(in crate::app) fn track_completion(&mut self) {
        let now = Instant::now();
        if !self.preedit_at(Focus::Editor).is_empty() {
            return;
        }
        if self.focus != Focus::Editor {
            self.completion.observe(None, now);
            return;
        }
        let enabled = self.vim_typing() && self.lsp.ready();
        let context = enabled
            .then(|| {
                self.editor
                    .active()
                    .filter(|doc| self.lsp.is_synced(doc.id))
                    .and_then(editor::completion::Completion::context)
            })
            .flatten();
        self.completion.observe(context, now);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use winc_core::Selection;
    use winc_core::edit::{Change, EditKind, Transaction};

    use super::*;

    fn unique_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("winc-lsp-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::write(path, []).unwrap();
    }

    /// 按 PATH、Go 的安装位置顺序查找；两处都没有时找不到。
    #[test]
    fn the_first_existing_server_wins() {
        let name = server_file_name("gopls");
        let path_dir = unique_dir("path");
        let go_dir = unique_dir("gobin");
        touch(&path_dir.join(&name));
        touch(&go_dir.join(&name));
        assert_eq!(
            first_existing(&[path_dir.clone(), go_dir.clone()], &name),
            Some(path_dir.join(&name)),
            "PATH 里的优先"
        );
        let empty = unique_dir("empty");
        assert_eq!(
            first_existing(&[empty, go_dir.clone()], &name),
            Some(go_dir.join(&name)),
            "PATH 里没有时找 Go 的安装位置"
        );
        assert_eq!(first_existing(&[unique_dir("none")], &name), None);
    }

    /// 用户配置的命令原样使用；置空则不启动；没写时按默认表查找。
    #[test]
    fn a_configured_command_wins_over_the_defaults() {
        let go_dir = unique_dir("default");
        let name = server_file_name("gopls");
        touch(&go_dir.join(&name));
        let config = winc_config::Config::default();
        assert_eq!(
            program_for(&config, "go", std::slice::from_ref(&go_dir)),
            Some(go_dir.join(&name))
        );
        let custom = unique_dir("custom").join(server_file_name("gopls-nightly"));
        let mut config = winc_config::Config::default();
        config
            .language_servers
            .insert("go".into(), custom.to_string_lossy().into_owned());
        assert_eq!(program_for(&config, "go", std::slice::from_ref(&go_dir)), Some(custom));
        config.language_servers.insert("go".into(), String::new());
        assert_eq!(program_for(&config, "go", std::slice::from_ref(&go_dir)), None);
        // 没有服务器的语言不启动。
        assert_eq!(program_for(&winc_config::Config::default(), "rust", &[]), None);
    }

    /// 单条改动按增量发（含删除多行时终点的行号）；多条改动或不支持增量时发全量。
    #[test]
    fn a_single_change_is_sent_incrementally() {
        let mut doc = Document::from_text(DocumentId(1), "abc\ndef");
        let before = doc.revision;
        doc.apply(Transaction::new(
            vec![Change { start: 1, removed: "b".into(), inserted: "BB".into() }],
            Selection::point(1),
            Selection::point(3),
            EditKind::Other,
        ));
        assert_eq!(
            content_changes(&doc, before, true),
            [ContentChange::Incremental {
                range: winc_lsp::message::Range {
                    start: winc_lsp::message::Position { line: 0, character: 1 },
                    end: winc_lsp::message::Position { line: 0, character: 2 },
                },
                text: "BB".into(),
            }]
        );

        // 删除 "b\ncd"：终点落在第二行的第二列。
        let mut doc = Document::from_text(DocumentId(2), "ab\ncd\nef");
        let before = doc.revision;
        doc.apply(Transaction::new(
            vec![Change { start: 1, removed: "b\ncd".into(), inserted: String::new() }],
            Selection::point(1),
            Selection::point(1),
            EditKind::Other,
        ));
        let ContentChange::Incremental { range, text } = &content_changes(&doc, before, true)[0]
        else {
            panic!("单条改动应是增量的");
        };
        assert_eq!(range.start, winc_lsp::message::Position { line: 0, character: 1 });
        assert_eq!(range.end, winc_lsp::message::Position { line: 1, character: 2 });
        assert_eq!(text, "");

        // 删除含换行的文本：终点按被删除的文本推进，而不是当前文本的同一偏移。
        let mut doc = Document::from_text(DocumentId(5), "ab
cd");
        let before = doc.revision;
        doc.apply(Transaction::new(
            vec![Change { start: 1, removed: "b
".into(), inserted: String::new() }],
            Selection::point(1),
            Selection::point(1),
            EditKind::Other,
        ));
        let ContentChange::Incremental { range, .. } = &content_changes(&doc, before, true)[0]
        else {
            panic!("单条改动应是增量的");
        };
        assert_eq!(range.start, winc_lsp::message::Position { line: 0, character: 1 });
        assert_eq!(range.end, winc_lsp::message::Position { line: 1, character: 0 });

        // 一次改动多处（多光标）：全量。
        let mut doc = Document::from_text(DocumentId(3), "abc");
        let before = doc.revision;
        doc.apply(Transaction::new(
            vec![
                Change { start: 0, removed: "a".into(), inserted: "x".into() },
                Change { start: 2, removed: "c".into(), inserted: "y".into() },
            ],
            Selection::point(0),
            Selection::point(0),
            EditKind::Other,
        ));
        assert_eq!(
            content_changes(&doc, before, true),
            [ContentChange::Full("xby".into())]
        );
        // 服务器不支持增量：全量。
        let mut doc = Document::from_text(DocumentId(4), "abc");
        let before = doc.revision;
        doc.apply(Transaction::new(
            vec![Change { start: 1, removed: "b".into(), inserted: "B".into() }],
            Selection::point(1),
            Selection::point(2),
            EditKind::Other,
        ));
        assert_eq!(
            content_changes(&doc, before, false),
            [ContentChange::Full("aBc".into())]
        );
    }

    /// 每帧的同步决策：关闭已关的、打开新开的、修订变化时发增量。
    #[test]
    fn sync_decisions_close_open_and_change() {
        let mut synced = HashMap::new();
        synced.insert(1, Synced { uri: "u1".into(), revision: 5 });
        synced.insert(3, Synced { uri: "u3".into(), revision: 2 });
        let docs = [
            SyncDoc { doc: DocumentId(1), revision: 6, uri: "u1".into() },
            SyncDoc { doc: DocumentId(2), revision: 1, uri: "u2".into() },
        ];
        let actions = sync_actions(&synced, &docs);
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0], SyncAction::Close { uri: "u3".into() });
        assert_eq!(actions[1], SyncAction::Open(docs[1].clone()));
        assert_eq!(
            actions[2],
            SyncAction::Change { doc: docs[0].clone(), previous: 5 }
        );
        // 没有变化时没有动作。
        let mut synced = HashMap::new();
        synced.insert(1, Synced { uri: "u1".into(), revision: 5 });
        let unchanged = [SyncDoc { doc: DocumentId(1), revision: 5, uri: "u1".into() }];
        assert!(sync_actions(&synced, &unchanged).is_empty());
    }

    /// 只同步本机、有对应服务器的文本文档：没有路径的标签、没有服务器的语言都不算。
    #[test]
    fn only_documents_with_a_server_are_synced() {
        let dir = unique_dir("docs");
        let go = dir.join("main.go");
        fs::write(&go, "package main
").unwrap();
        let md = dir.join("a.md");
        fs::write(&md, "# 标题
").unwrap();
        let mut editor = Editor::default();
        editor.workspace = Some(winc_editor::Workspace::open(&dir).unwrap());
        editor.open(&go);
        editor.open(&md);
        editor.show_output("输出", "内容");
        let app = App::new(editor, None, Bench::from_env(), winc_config::Config::default());
        let docs = app.server_documents();
        assert_eq!(docs.len(), 1, "只有 Go 文档");
        assert_eq!(docs[0].1, "go");
        // 没有打开目录时没有工作区根，调用方因此不会启动服务器。
        let mut editor = Editor::default();
        editor.open(&go);
        let app = App::new(editor, None, Bench::from_env(), winc_config::Config::default());
        assert!(app.editor.root().is_none());
    }

    /// 崩溃后 1 分钟内再次崩溃：不再自动重启。
    #[test]
    fn a_crash_loop_stops_restarting() {
        let now = Instant::now();
        let mut state = LspState::default();
        assert!(state.crash(now), "第一次崩溃允许重启");
        assert!(
            !state.crash(now + Duration::from_secs(30)),
            "1 分钟内再次崩溃不再重启"
        );
        assert!(state.crash(now + Duration::from_secs(120)));
    }

    /// 全部文档关闭后开始空闲计时；重新打开文档后取消。
    #[test]
    fn idle_counts_from_the_last_closed_document() {
        let now = Instant::now();
        let mut state = LspState::default();
        state.note_documents(0, now);
        assert_eq!(state.idle_deadline(), Some(now + IDLE_TIMEOUT));
        state.note_documents(1, now + Duration::from_secs(10));
        assert_eq!(state.idle_deadline(), None);
        state.note_documents(0, now + Duration::from_secs(20));
        assert_eq!(
            state.idle_deadline(),
            Some(now + Duration::from_secs(20) + IDLE_TIMEOUT)
        );
    }

    /// 后台启动失败：记为不可用，且本会话不再尝试；配置改了命令之后可以重试。
    #[test]
    fn a_failed_start_marks_the_session_unavailable_until_the_config_changes() {
        let mut state = LspState::default();
        assert!(state.connection.is_none());
        let slot = state
            .begin_start("go", "gopls", None)
            .expect("可以开始启动");
        let result = Connection::start(Path::new("winc-不存在的语言服务器"), || {});
        assert!(result.is_err());
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        let Adopted::Failed(reason) = state.adopt_start() else {
            panic!("应当启动失败");
        };
        assert!(!reason.is_empty());
        assert!(state.connection.is_none(), "启动失败不该有连接");
        assert!(state.begin_start("go", "gopls", None).is_none(), "本会话不再自动启动");

        // 用户改了命令：允许重试。
        let mut config = winc_config::Config::default();
        config.language_servers.insert("go".into(), "nightly-gopls".into());
        state.recheck_config(&config);
        assert!(state.unavailable().is_none());
        assert!(state.begin_start("go", "nightly-gopls", None).is_some());
    }

    /// 没有连接时轮询返回空、发送丢弃，不崩溃。
    #[test]
    fn polling_and_sending_without_a_connection_do_nothing() {
        let mut state = LspState::default();
        assert!(matches!(state.poll(), Poll::Empty));
        state.send(&Message::Notification { method: "exit".into(), params: None });
        assert!(state.connection.is_none());
    }
}
