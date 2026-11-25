//! LSP 消息子集：本 crate 支持的请求、通知与响应的类型，及与 `Json` 的双向转换。
//!
//! 只建模补全与诊断所需的字段：LSP 的多数消息有大量可选字段，这里只保留会用到的，
//! 其余字段解析时忽略、序列化时不写。客户端发给服务器的消息实现 `to_json`，服务器发回的
//! 只实现 `from_json`（解析不了的报文按「无法理解就丢弃」处理，与 `winc-json` 的失败
//! 语义一致）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use winc_json::Json;

/// JSON-RPC 方法名。
pub mod method {
    /// 握手：交换双方能力。
    pub const INITIALIZE: &str = "initialize";
    /// 握手完成通知。
    pub const INITIALIZED: &str = "initialized";
    /// 关闭请求。
    pub const SHUTDOWN: &str = "shutdown";
    /// 关闭后退出通知。
    pub const EXIT: &str = "exit";
    /// 打开文档通知。
    pub const DID_OPEN: &str = "textDocument/didOpen";
    /// 文档变更通知。
    pub const DID_CHANGE: &str = "textDocument/didChange";
    /// 关闭文档通知。
    pub const DID_CLOSE: &str = "textDocument/didClose";
    /// 补全请求。
    pub const COMPLETION: &str = "textDocument/completion";
    /// 诊断推送通知（服务器主动发）。
    pub const PUBLISH_DIAGNOSTICS: &str = "textDocument/publishDiagnostics";
    /// 服务器消息通知（只记录）。
    pub const SHOW_MESSAGE: &str = "window/showMessage";
}

/// 从对象里读一个字符串字段；不是字符串时返回 `None`。
fn str_field(map: &BTreeMap<String, Json>, key: &str) -> Option<String> {
    map.get(key).and_then(Json::as_str).map(str::to_owned)
}

/// 从对象里读一个非负整数字段。
fn u32_field(map: &BTreeMap<String, Json>, key: &str) -> Option<u32> {
    map.get(key).and_then(Json::as_number).map(|n| n as u32)
}

/// 从对象里读一个整数字段（文档版本号用，服务器不会发小数）。
fn i32_field(map: &BTreeMap<String, Json>, key: &str) -> Option<i32> {
    map.get(key).and_then(Json::as_number).map(|n| n as i64 as i32)
}

/// 从对象里读一个布尔字段。
fn bool_field(map: &BTreeMap<String, Json>, key: &str) -> Option<bool> {
    map.get(key).and_then(Json::as_bool)
}

/// 从对象里读一个数组字段。
fn array_field<'a>(map: &'a BTreeMap<String, Json>, key: &str) -> Option<&'a [Json]> {
    map.get(key).and_then(Json::as_array)
}

/// 从对象里读一个对象字段。
fn object_field<'a>(
    map: &'a BTreeMap<String, Json>,
    key: &str,
) -> Option<&'a BTreeMap<String, Json>> {
    map.get(key).and_then(Json::as_object)
}

/// 数字字面量（整数）。
fn num(value: u32) -> Json {
    Json::Number(value as f64)
}

/// 文档版本号字面量（`i32`，LSP 规定）。
fn version_num(value: i32) -> Json {
    Json::Number(value as f64)
}

/// LSP 位置：行号 + 行内 UTF-16 code unit 数（协议默认编码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// 行号，从 0 起。
    pub line: u32,
    /// 行内位置，UTF-16 code unit 数，从 0 起。
    pub character: u32,
}

impl Position {
    /// 解析 `{"line": L, "character": C}`。
    pub fn from_json(json: &Json) -> Option<Position> {
        let map = json.as_object()?;
        Some(Position {
            line: u32_field(map, "line")?,
            character: u32_field(map, "character")?,
        })
    }

    /// 写出 `{"line": L, "character": C}`。
    pub fn to_json(&self) -> Json {
        Json::object([("line", num(self.line)), ("character", num(self.character))])
    }
}

/// LSP 范围：从 [`Position::start`] 到 [`Position::end`]，半开区间（不含 end 处字符）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// 范围起点。
    pub start: Position,
    /// 范围终点（不含）。
    pub end: Position,
}

impl Range {
    /// 解析 `{"start": {...}, "end": {...}}`。
    pub fn from_json(json: &Json) -> Option<Range> {
        let map = json.as_object()?;
        Some(Range {
            start: Position::from_json(map.get("start")?)?,
            end: Position::from_json(map.get("end")?)?,
        })
    }

    /// 写出 `{"start": {...}, "end": {...}}`。
    pub fn to_json(&self) -> Json {
        Json::object([("start", self.start.to_json()), ("end", self.end.to_json())])
    }
}

/// 文本编辑：把 `range` 内的文本替换为 `new_text`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// 被替换的范围。
    pub range: Range,
    /// 替换后的文本。
    pub new_text: String,
}

impl TextEdit {
    /// 解析 `{"range": {...}, "newText": "..."}`。
    pub fn from_json(json: &Json) -> Option<TextEdit> {
        let map = json.as_object()?;
        Some(TextEdit {
            range: Range::from_json(map.get("range")?)?,
            new_text: str_field(map, "newText")?,
        })
    }

    /// 写出 `{"range": {...}, "newText": "..."}`。
    pub fn to_json(&self) -> Json {
        Json::object([("range", self.range.to_json()), ("newText", Json::String(self.new_text.clone()))])
    }
}

/// 诊断严重级别。LSP 还有 Information 与 Hint（3、4），渲染上与警告同色，这里并成两级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 错误（LSP 的 1）。
    Error,
    /// 警告（LSP 的 2，3、4 也按此处理）。
    Warning,
}

impl Severity {
    /// 从 LSP 的数字解析：1 为错误，其余（含缺失）按警告。
    fn from_code(code: u32) -> Severity {
        if code == 1 { Severity::Error } else { Severity::Warning }
    }

    /// 写回 LSP 的数字。
    fn to_code(self) -> u32 {
        match self {
            Severity::Error => 1,
            Severity::Warning => 2,
        }
    }
}

/// 一条诊断：范围内的错误或警告消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 诊断覆盖的范围。
    pub range: Range,
    /// 严重级别。
    pub severity: Severity,
    /// 消息文本。
    pub message: String,
}

impl Diagnostic {
    /// 解析诊断对象；`severity` 缺失时按 LSP 规定视为错误，`code`、`source` 等忽略。
    pub fn from_json(json: &Json) -> Option<Diagnostic> {
        let map = json.as_object()?;
        Some(Diagnostic {
            range: Range::from_json(map.get("range")?)?,
            severity: Severity::from_code(u32_field(map, "severity").unwrap_or(1)),
            message: str_field(map, "message")?,
        })
    }

    /// 写出诊断对象（只写本 crate 用到的字段）。
    pub fn to_json(&self) -> Json {
        Json::object([
            ("range", self.range.to_json()),
            ("severity", num(self.severity.to_code())),
            ("message", Json::String(self.message.clone())),
        ])
    }
}

/// 一条补全候选。接受时优先用 `text_edit`，其次 `insert_text`，都没有时用 `label`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// 显示与回退插入的文本。
    pub label: String,
    /// 候选种类（LSP 的 `CompletionItemKind` 数字），用于列表的种类文字。
    pub kind: Option<u32>,
    /// 排序键；缺失时用 `label`。
    pub sort_text: Option<String>,
    /// 过滤键；缺失时用 `label`。
    pub filter_text: Option<String>,
    /// 插入文本（无 `text_edit` 时用）。
    pub insert_text: Option<String>,
    /// 带范围的精确编辑（有 `additional_text_edits` 之外的替换范围时用）。
    pub text_edit: Option<TextEdit>,
    /// 接受时一并应用的附加编辑（例如自动补 import 行）。
    pub additional_text_edits: Vec<TextEdit>,
}

impl CompletionItem {
    /// 解析补全候选；`detail`、`documentation` 等展示字段忽略。
    pub fn from_json(json: &Json) -> Option<CompletionItem> {
        let map = json.as_object()?;
        Some(CompletionItem {
            label: str_field(map, "label")?,
            kind: u32_field(map, "kind"),
            sort_text: str_field(map, "sortText"),
            filter_text: str_field(map, "filterText"),
            insert_text: str_field(map, "insertText"),
            text_edit: map.get("textEdit").and_then(TextEdit::from_json),
            additional_text_edits: array_field(map, "additionalTextEdits")
                .unwrap_or(&[])
                .iter()
                .filter_map(TextEdit::from_json)
                .collect(),
        })
    }
}

/// 补全响应里的列表：`is_incomplete` 为真时列表只是前缀匹配的一部分，继续输入应重新请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionList {
    /// 是否不完整。
    pub is_incomplete: bool,
    /// 候选列表。
    pub items: Vec<CompletionItem>,
}

/// 补全响应：对象形式（带 `isIncomplete`）或纯数组形式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionResponse {
    /// 列表形式。
    List(CompletionList),
    /// 纯数组形式。
    Items(Vec<CompletionItem>),
}

impl CompletionResponse {
    /// 解析两种形式的补全响应。
    pub fn from_json(json: &Json) -> Option<CompletionResponse> {
        if let Some(map) = json.as_object() {
            return Some(CompletionResponse::List(CompletionList {
                is_incomplete: bool_field(map, "isIncomplete").unwrap_or(false),
                items: array_field(map, "items")
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(CompletionItem::from_json)
                    .collect(),
            }));
        }
        json.as_array()
            .map(|items| CompletionResponse::Items(items.iter().filter_map(CompletionItem::from_json).collect()))
    }

    /// 全部候选（两种形式统一取）。
    pub fn items(&self) -> &[CompletionItem] {
        match self {
            CompletionResponse::List(list) => &list.items,
            CompletionResponse::Items(items) => items,
        }
    }
}

/// 握手请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitializeParams {
    /// 客户端进程号；服务器只用于日志。
    pub process_id: u32,
    /// 工作区根的 file URI；散开的单文件没有根。
    pub root_uri: Option<String>,
}

impl InitializeParams {
    /// 写出参数。客户端能力只声明补全；诊断推送不需要客户端能力。
    pub fn to_json(&self) -> Json {
        Json::object([
            ("processId", Json::Number(self.process_id as f64)),
            (
                "rootUri",
                match &self.root_uri {
                    Some(uri) => Json::String(uri.clone()),
                    None => Json::Null,
                },
            ),
            (
                "capabilities",
                Json::object([(
                    "textDocument",
                    Json::object([("completion", Json::Object(BTreeMap::new()))]),
                )]),
            ),
        ])
    }
}

/// 握手响应里我们关心的服务器能力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCapabilities {
    /// 文档同步方式是否为增量（LSP 的 2）；否则按全量。
    pub incremental: bool,
    /// 补全触发字符（gopls 是 `.`）。
    pub completion_trigger_characters: Vec<String>,
}

impl ServerCapabilities {
    /// 从握手响应里解析；`textDocumentSync` 有数字（1、2）与对象（`change` 字段）两种写法。
    pub fn from_json(json: &Json) -> Option<ServerCapabilities> {
        let map = object_field(json.as_object()?, "capabilities")?;
        let sync = match map.get("textDocumentSync") {
            Some(Json::Number(n)) => Some(*n as u32),
            Some(object) => object.as_object().and_then(|m| u32_field(m, "change")),
            _ => None,
        };
        Some(ServerCapabilities {
            incremental: sync == Some(2),
            completion_trigger_characters: map
                .get("completionProvider")
                .and_then(Json::as_object)
                .and_then(|m| array_field(m, "triggerCharacters"))
                .map(|chars| {
                    chars
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// 打开文档通知的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidOpenParams {
    /// 文档的 file URI。
    pub uri: String,
    /// 语言 id（如 `go`）。
    pub language_id: String,
    /// 文档版本（从 1 起单调递增；这里传文档的 revision）。
    pub version: i32,
    /// 全文。
    pub text: String,
}

impl DidOpenParams {
    /// 写出参数。
    pub fn to_json(&self) -> Json {
        Json::object([(
            "textDocument",
            Json::object([
                ("uri", Json::String(self.uri.clone())),
                ("languageId", Json::String(self.language_id.clone())),
                ("version", version_num(self.version)),
                ("text", Json::String(self.text.clone())),
            ]),
        )])
    }
}

/// 一条文档变更：全量替换，或 `range` 内文本替换为 `text`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentChange {
    /// 全文替换（不带 `range`）。
    Full(String),
    /// 增量替换：`range` 内的文本换成 `text`。
    Incremental {
        /// 被替换的范围。
        range: Range,
        /// 替换后的文本。
        text: String,
    },
}

impl ContentChange {
    /// 写出变更（有范围为增量，无范围为全量）。
    fn to_json(&self) -> Json {
        match self {
            ContentChange::Full(text) => Json::object([("text", Json::String(text.clone()))]),
            ContentChange::Incremental { range, text } => Json::object([
                ("range", range.to_json()),
                ("text", Json::String(text.clone())),
            ]),
        }
    }
}

/// 文档变更通知的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidChangeParams {
    /// 文档的 file URI。
    pub uri: String,
    /// 变更后的版本号。
    pub version: i32,
    /// 变更列表（服务器按顺序应用）。
    pub changes: Vec<ContentChange>,
}

impl DidChangeParams {
    /// 写出参数。
    pub fn to_json(&self) -> Json {
        Json::object([
            (
                "textDocument",
                Json::object([
                    ("uri", Json::String(self.uri.clone())),
                    ("version", version_num(self.version)),
                ]),
            ),
            (
                "contentChanges",
                Json::Array(self.changes.iter().map(ContentChange::to_json).collect()),
            ),
        ])
    }
}

/// 关闭文档通知的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidCloseParams {
    /// 文档的 file URI。
    pub uri: String,
}

impl DidCloseParams {
    /// 写出参数。
    pub fn to_json(&self) -> Json {
        Json::object([("textDocument", Json::object([("uri", Json::String(self.uri.clone()))]))])
    }
}

/// 补全触发方式（LSP 的 `CompletionTriggerKind` 数字，只透传）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionContext {
    /// 触发种类数字：1 手动、2 触发字符、3 不完整列表续填。
    pub trigger_kind: u32,
    /// 触发字符（如 `.`）。
    pub trigger_character: Option<String>,
}

impl CompletionContext {
    /// 写出上下文。
    fn to_json(&self) -> Json {
        Json::object([
            ("triggerKind", num(self.trigger_kind)),
            (
                "triggerCharacter",
                match &self.trigger_character {
                    Some(c) => Json::String(c.clone()),
                    None => Json::Null,
                },
            ),
        ])
    }
}

/// 补全请求的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionParams {
    /// 文档的 file URI。
    pub uri: String,
    /// 请求补全的位置（光标）。
    pub position: Position,
    /// 触发上下文；可省略（按手动触发发）。
    pub context: Option<CompletionContext>,
}

impl CompletionParams {
    /// 写出参数。
    pub fn to_json(&self) -> Json {
        let mut fields = vec![
            (
                "textDocument",
                Json::object([("uri", Json::String(self.uri.clone()))]),
            ),
            ("position", self.position.to_json()),
        ];
        if let Some(context) = &self.context {
            fields.push(("context", context.to_json()));
        }
        Json::object(fields)
    }
}

/// 诊断推送通知的参数（服务器主动发）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishDiagnosticsParams {
    /// 文档的 file URI。
    pub uri: String,
    /// 诊断对应的文档版本；服务器可能不给。
    pub version: Option<i32>,
    /// 该文档的全部诊断（整表替换，不是追加）。
    pub diagnostics: Vec<Diagnostic>,
}

impl PublishDiagnosticsParams {
    /// 解析参数。
    pub fn from_json(json: &Json) -> Option<PublishDiagnosticsParams> {
        let map = json.as_object()?;
        Some(PublishDiagnosticsParams {
            uri: str_field(map, "uri")?,
            version: i32_field(map, "version"),
            diagnostics: array_field(map, "diagnostics")
                .unwrap_or(&[])
                .iter()
                .filter_map(Diagnostic::from_json)
                .collect(),
        })
    }
}

/// 服务器消息通知的参数（只记录文本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowMessageParams {
    /// 消息文本。
    pub message: String,
}

impl ShowMessageParams {
    /// 解析参数；`type`（1 错误到 4 提示）不区分，都只记录。
    pub fn from_json(json: &Json) -> Option<ShowMessageParams> {
        let map = json.as_object()?;
        Some(ShowMessageParams {
            message: str_field(map, "message")?,
        })
    }
}

/// 把本机路径转成 file URI：`file:///D:/code/a%20b.go`、`file:///home/x/main.go`。
///
/// 分隔统一为 `/`，非 URI 安全字符按 UTF-8 百分号编码（空格、中文、`#` 等）。相对路径与
/// 非 UTF-8 路径返回 `None`（没有根的文档进不了 LSP）。
pub fn file_uri(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let parts: Vec<&str> = text.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    let drive = parts.first().copied().filter(|first| {
        let bytes = first.as_bytes();
        bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
    });
    let Some(drive) = drive else {
        // Unix 根（`/home/…`）：全部组件编码。相对路径没有根，不能作为文档 URI。
        if !text.starts_with('/') {
            return None;
        }
        let encoded = parts.iter().map(|part| encode_component(part)).collect::<Vec<_>>();
        let mut out = String::from("file:///");
        out.push_str(&encoded.join("/"));
        return Some(out);
    };
    // Windows 盘符：`D:` 原样（`:` 是 URI 允许的保留字符），其余组件编码。
    let rest = parts[1..].iter().map(|part| encode_component(part)).collect::<Vec<_>>();
    let mut out = format!("file:///{drive}");
    if !rest.is_empty() {
        out.push('/');
        out.push_str(&rest.join("/"));
    }
    Some(out)
}

/// 路径组件的百分号编码：保留字母数字与 `-._~`，其余按 UTF-8 字节编码。
fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for &byte in text.as_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 把 file URI 转回本机路径；协议不是 `file`、带主机名或编码非法时返回 `None`。
///
/// 接受 `file:///D:/…`（Windows）、`file:///home/…`（Unix）与省掉第三个斜杠的
/// `file://D:/…`。分隔符保持 `/`（与 URI 一致，仅用于显示与日志）。
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        // `file:///…`：看斜杠后是不是 Windows 盘符，不是就按 Unix 根路径原样取。
        let after_slash = rest.strip_prefix('/').unwrap_or(rest);
        let bytes = after_slash.as_bytes();
        let is_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        if is_drive { after_slash } else { rest }
    } else {
        // `file://D:/…`：省掉第三个斜杠的 Windows 写法；其余是主机名（不支持）。
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            rest
        } else {
            return None;
        }
    };

    let bytes = path.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let hex = bytes.get(i + 1..i + 3)?;
        out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
        i += 3;
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests;
