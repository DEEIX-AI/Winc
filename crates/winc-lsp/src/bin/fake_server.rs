//! 假语言服务器：只给 `tests/lifecycle.rs` 用，复用本 crate 的分帧与报文信封。
//!
//! 剧本：应答 `initialize`（声明增量同步与 `.` 触发补全）；收到 `initialized` 后先发一个
//! 带两个 items 的 `workspace/configuration` 请求，收到长度匹配的 null 数组应答才推送一份
//! 固定的诊断（错误、警告各一）——这让「连接自动应答了服务器请求」成为可观察行为；
//! `didOpen` 按报文里的 uri 与版本推一条诊断；`completion` 回三个固定候选；`shutdown`
//! 回 null；`exit` 在应答过 shutdown 后以 0 退出。不认识的请求回方法不存在。

use std::io::{Read, Write};

use winc_json::Json;
use winc_lsp::message::{Diagnostic, Position, Range, Severity, method};
use winc_lsp::rpc::{self, Id, Message};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut server = Server { writer: stdout.lock(), pushed: false, shut_down: false };
    let mut decoder = rpc::Decoder::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        decoder.push(&buf[..n]);
        while let Some(message) = decoder.next_message() {
            server.handle(message);
        }
    }
}

/// 会话状态：是否已按 initialized 剧本推过诊断、是否应答过 shutdown。
struct Server<W: Write> {
    writer: W,
    pushed: bool,
    shut_down: bool,
}

impl<W: Write> Server<W> {
    fn handle(&mut self, message: Message) {
        match message {
            Message::Request { id, method, params } => self.request(id, method, params),
            Message::Notification { method, params } => self.notification(method, params),
            Message::Response { id, result, .. } => self.response(id, result),
        }
    }

    fn request(&mut self, id: Id, name: String, params: Option<Json>) {
        let _ = params;
        match name.as_str() {
            "initialize" => {
                let capabilities = Json::object([
                    (
                        "textDocumentSync",
                        Json::object([
                            ("openClose", Json::Bool(true)),
                            ("change", Json::Number(2.0)),
                        ]),
                    ),
                    (
                        "completionProvider",
                        Json::object([(
                            "triggerCharacters",
                            Json::Array(vec![Json::String(".".into())]),
                        )]),
                    ),
                ]);
                // 真实服务器的 result 里包一层 `capabilities`。
                let result = Json::object([("capabilities", capabilities)]);
                self.send(&Message::Response { id, result: Some(result), error: None });
            }
            "textDocument/completion" => {
                let items = ["Print", "Printf", "Println"].map(|label| {
                    Json::object([
                        ("label", Json::String(label.into())),
                        ("kind", Json::Number(3.0)),
                        ("sortText", Json::String("00000".into())),
                        ("insertText", Json::String(label.into())),
                    ])
                });
                let list = Json::object([
                    ("isIncomplete", Json::Bool(true)),
                    ("items", Json::Array(items.to_vec())),
                ]);
                self.send(&Message::Response { id, result: Some(list), error: None });
            }
            "shutdown" => {
                self.shut_down = true;
                self.send(&Message::Response { id, result: Some(Json::Null), error: None });
            }
            _ => {
                self.send(&Message::Response {
                    id,
                    result: None,
                    error: Some(rpc::RpcError { code: -32601, message: "method not found".into() }),
                });
            }
        }
    }

    fn notification(&mut self, name: String, params: Option<Json>) {
        match name.as_str() {
            method::INITIALIZED => self.initialized(),
            method::DID_OPEN => self.did_open(params),
            method::EXIT => std::process::exit(if self.shut_down { 0 } else { 1 }),
            _ => {}
        }
    }

    fn response(&mut self, id: Id, result: Option<Json>) {
        // 只关心对 configuration 请求的应答：长度匹配的 null 数组才算通过，然后推诊断。
        if id != Id::Number(100) || self.pushed {
            return;
        }
        let correct = result
            .as_ref()
            .and_then(Json::as_array)
            .is_some_and(|items| items.len() == 2 && items.iter().all(|item| *item == Json::Null));
        if correct {
            self.pushed = true;
            self.push_diagnostics(
                "file:///test/main.go",
                None,
                vec![
                    Diagnostic {
                        range: Range {
                            start: Position { line: 0, character: 1 },
                            end: Position { line: 0, character: 2 },
                        },
                        severity: Severity::Error,
                        message: "undefined: x".into(),
                    },
                    Diagnostic {
                        range: Range {
                            start: Position { line: 1, character: 0 },
                            end: Position { line: 1, character: 3 },
                        },
                        severity: Severity::Warning,
                        message: "unused variable".into(),
                    },
                ],
            );
        }
    }

    /// initialized 的剧本：先发服务器请求，诊断在收到正确的自动应答后推。
    fn initialized(&mut self) {
        self.send(&Message::Request {
            id: Id::Number(100),
            method: "workspace/configuration".into(),
            params: Some(Json::object([(
                "items",
                Json::Array(vec![
                    Json::object([("section", Json::String("gopls".into()))]),
                    Json::object([("section", Json::String("gopls".into()))]),
                ]),
            )])),
        });
    }

    fn did_open(&mut self, params: Option<Json>) {
        let Some(document) = params
            .as_ref()
            .and_then(Json::as_object)
            .and_then(|map| map.get("textDocument"))
            .and_then(Json::as_object)
        else {
            return;
        };
        let Some(uri) = document.get("uri").and_then(Json::as_str) else { return };
        let version = document.get("version").and_then(Json::as_number).map(|n| n as i32);
        self.push_diagnostics(
            uri,
            version,
            vec![Diagnostic {
                range: Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 1 },
                },
                severity: Severity::Error,
                message: "did open".into(),
            }],
        );
    }

    fn push_diagnostics(&mut self, uri: &str, version: Option<i32>, diagnostics: Vec<Diagnostic>) {
        let items = Json::Array(diagnostics.iter().map(Diagnostic::to_json).collect());
        let mut fields = vec![("uri", Json::String(uri.into()))];
        if let Some(version) = version {
            fields.push(("version", Json::Number(version as f64)));
        }
        fields.push(("diagnostics", items));
        self.send(&Message::Notification {
            method: method::PUBLISH_DIAGNOSTICS.into(),
            params: Some(Json::object(fields)),
        });
    }

    fn send(&mut self, message: &Message) {
        let bytes = rpc::frame(message);
        let _ = self.writer.write_all(&bytes);
        let _ = self.writer.flush();
    }
}
