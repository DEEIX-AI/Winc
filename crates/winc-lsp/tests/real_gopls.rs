//! 真实 gopls 的集成验收：需要本机装有 `gopls`（`PATH` 或 `%USERPROFILE%/go/bin`），
//! 没有时跳过。覆盖握手、文档同步、诊断与补全——与手动验收（tasks 9.2）对应的可重复部分。

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use winc_json::Json;
use winc_lsp::connection::{Connection, Poll};
use winc_lsp::message::{
    self, CompletionContext, CompletionParams, CompletionResponse, ContentChange, DidChangeParams,
    DidOpenParams, InitializeParams, PublishDiagnosticsParams, Range, ServerCapabilities, method,
};
use winc_lsp::position;
use winc_lsp::rpc::{Id, Message};

/// 找本机的 gopls；没有时返回 `None`（测试跳过）。
fn gopls() -> Option<PathBuf> {
    let name = if cfg!(windows) { "gopls.exe" } else { "gopls" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(PathBuf::from(home).join("go").join("bin"));
    }
    dirs.iter().map(|dir| dir.join(name)).find(|path| path.is_file())
}

fn next(connection: &Connection, woken: &Receiver<()>, timeout: Duration) -> Option<Message> {
    let until = Instant::now() + timeout;
    loop {
        match connection.poll() {
            Poll::Message(message) => return Some(message),
            Poll::Closed => return None,
            Poll::Empty => {
                let left = until.checked_duration_since(Instant::now())?;
                if woken.recv_timeout(left).is_err() {
                    return None;
                }
            }
        }
    }
}

/// 等到 `method` 的通知；期间收到的其他报文忽略。
fn wait_notification(
    connection: &Connection,
    woken: &Receiver<()>,
    method: &str,
    timeout: Duration,
) -> Option<Json> {
    let until = Instant::now() + timeout;
    loop {
        let left = until.checked_duration_since(Instant::now())?;
        let message = next(connection, woken, left)?;
        if let Message::Notification { method: name, params } = message
            && name == method
        {
            return params;
        }
    }
}

#[test]
fn a_real_gopls_session_reports_diagnostics_and_completions() {
    let Some(program) = gopls() else {
        eprintln!("跳过：本机没有 gopls");
        return;
    };
    let dir = std::env::temp_dir().join(format!("winc-gopls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("go.mod"), "module demo\n\ngo 1.22\n").unwrap();
    let text = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tvar r Reader\n\tfmt.Println(r.Fi)\n}\n\ntype Reader struct {\n\tField int\n}\n\nfunc (r Reader) ReadAll() int { return r.Field }\n";
    let file = dir.join("main.go");
    std::fs::write(&file, text).unwrap();
    // 目录是新建的临时目录，没有符号链接，原样作为根即可。
    let root = dir.clone();

    let (tx, rx) = channel();
    let connection = Connection::start(&program, move || {
        let _ = tx.send(());
    })
    .expect("gopls 能启动");
    let send = |message: Message| connection.send(&message);

    // 握手。
    send(Message::Request {
        id: Id::Number(1),
        method: method::INITIALIZE.into(),
        params: Some(
            InitializeParams {
                process_id: std::process::id(),
                root_uri: message::file_uri(&root),
            }
            .to_json(),
        ),
    });
    let capabilities = loop {
        let Some(message) = next(&connection, &rx, Duration::from_secs(60)) else {
            panic!("gopls 未在 60 秒内应答 initialize");
        };
        if let Message::Response { id: Id::Number(1), result: Some(json), .. } = message {
            break ServerCapabilities::from_json(&json).expect("能力能解析");
        }
    };
    assert!(capabilities.incremental, "gopls 支持增量同步");
    assert_eq!(capabilities.completion_trigger_characters, ["."]);
    send(Message::Notification {
        method: method::INITIALIZED.into(),
        params: Some(Json::Null),
    });

    // 打开文档：gopls 会在首次分析完成后推诊断。
    let uri = message::file_uri(&file).expect("file uri");
    send(Message::Notification {
        method: method::DID_OPEN.into(),
        params: Some(
            DidOpenParams {
                uri: uri.clone(),
                language_id: "go".into(),
                version: 1,
                text: text.into(),
            }
            .to_json(),
        ),
    });
    let params = wait_notification(
        &connection,
        &rx,
        method::PUBLISH_DIAGNOSTICS,
        Duration::from_secs(120),
    )
    .expect("首次分析后会推送诊断");
    let diagnostics = PublishDiagnosticsParams::from_json(&params).expect("诊断能解析");
    assert!(
        diagnostics.diagnostics.iter().any(|d| d.message.contains("Fi")),
        "应有 r.Fi 的错误：{:?}",
        diagnostics.diagnostics
    );

    // 补全：`r.` 之后请求成员。
    let caret = text.find("r.Fi").unwrap() + 2;
    let line = text[..caret].matches('\n').count();
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let line_text = text[line_start..].lines().next().unwrap_or("");
    let at = position::position_at(line as u32, line_text, caret - line_start);
    send(Message::Request {
        id: Id::Number(2),
        method: method::COMPLETION.into(),
        params: Some(
            CompletionParams {
                uri: uri.clone(),
                position: at,
                context: Some(CompletionContext {
                    trigger_kind: 2,
                    trigger_character: Some(".".into()),
                }),
            }
            .to_json(),
        ),
    });
    let items = loop {
        let Some(message) = next(&connection, &rx, Duration::from_secs(60)) else {
            panic!("gopls 未在 60 秒内应答补全");
        };
        if let Message::Response { id: Id::Number(2), result: Some(json), .. } = message {
            break CompletionResponse::from_json(&json)
                .map(|r| r.items().to_vec())
                .unwrap_or_default();
        }
    };
    let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
    assert!(labels.contains(&"Field"), "成员补全里有 Field：{labels:?}");
    assert!(labels.contains(&"ReadAll"), "成员补全里有 ReadAll：{labels:?}");

    // 增量修改：把 `r.Fi` 换成 `r.Field`，gopls 应清掉该诊断（再推一次为空）。
    let after = position::position_at(line as u32, line_text, caret - line_start + 2);
    send(Message::Notification {
        method: method::DID_CHANGE.into(),
        params: Some(
            DidChangeParams {
                uri: uri.clone(),
                version: 2,
                changes: vec![ContentChange::Incremental {
                    range: Range { start: at, end: after },
                    text: "Field".into(),
                }],
            }
            .to_json(),
        ),
    });
    let mut cleared = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let left = deadline - Instant::now();
        let Some(params) = wait_notification(&connection, &rx, method::PUBLISH_DIAGNOSTICS, left)
        else {
            break;
        };
        let params = PublishDiagnosticsParams::from_json(&params).unwrap();
        if params.diagnostics.is_empty() {
            cleared = true;
            break;
        }
    }
    assert!(cleared, "修正后诊断应清空");

    // 关闭：shutdown → exit。
    send(Message::Request {
        id: Id::Number(3),
        method: method::SHUTDOWN.into(),
        params: None,
    });
    send(Message::Notification {
        method: method::EXIT.into(),
        params: None,
    });
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);
}
