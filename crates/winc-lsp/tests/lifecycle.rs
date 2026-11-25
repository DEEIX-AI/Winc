//! 连接的生命周期集成测试：用假服务器（`src/bin/fake_server.rs`）跑完整的
//! 握手 → 服务器请求自动应答 → 文档同步 → 补全 → 关闭流程，外加启动失败与崩溃两条
//! 降级路径。

use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use winc_json::Json;
use winc_lsp::connection::{Connection, Poll};
use winc_lsp::message::{
    CompletionParams, CompletionResponse, DidOpenParams, InitializeParams, Position,
    PublishDiagnosticsParams, ServerCapabilities, Severity, method,
};
use winc_lsp::rpc::{Id, Message};

/// 等待并取一条报文；10 秒等不到唤醒就失败（假服务器不应卡住）。
fn next(connection: &Connection, woken: &Receiver<()>) -> Message {
    loop {
        match connection.poll() {
            Poll::Message(message) => return message,
            Poll::Empty => woken.recv_timeout(Duration::from_secs(10)).expect("等唤醒超时"),
            Poll::Closed => panic!("连接提前断开"),
        }
    }
}

/// 等连接报告断开；断开前再收到报文视为失败。
fn closed(connection: &Connection, woken: &Receiver<()>) {
    loop {
        match connection.poll() {
            Poll::Closed => return,
            Poll::Message(message) => panic!("断开前不该再有报文：{message:?}"),
            Poll::Empty => woken.recv_timeout(Duration::from_secs(10)).expect("等唤醒超时"),
        }
    }
}

/// 启动假服务器并配好唤醒通道。
fn start_fake() -> (Connection, Receiver<()>) {
    let (tx, rx) = channel();
    let connection = Connection::start(Path::new(env!("CARGO_BIN_EXE_fake_server")), move || {
        let _ = tx.send(());
    })
    .expect("假服务器能启动");
    (connection, rx)
}

#[test]
fn handshake_document_sync_completion_and_shutdown() {
    let (connection, rx) = start_fake();

    // 握手：服务器应回增量同步与补全触发字符。
    connection.send(&Message::Request {
        id: Id::Number(1),
        method: method::INITIALIZE.into(),
        params: Some(
            InitializeParams {
                process_id: std::process::id(),
                root_uri: Some("file:///test".into()),
            }
            .to_json(),
        ),
    });
    let capabilities = match next(&connection, &rx) {
        Message::Response { result: Some(json), .. } => {
            ServerCapabilities::from_json(&json).expect("能力能解析")
        }
        other => panic!("应是握手响应：{other:?}"),
    };
    assert!(capabilities.incremental);
    assert_eq!(capabilities.completion_trigger_characters, ["."]);

    // initialized 之后：假服务器先发 configuration 请求，连接自动应答 null 数组，
    // 应答正确它才推诊断——诊断到了就证明自动应答工作正常。
    connection.send(&Message::Notification {
        method: method::INITIALIZED.into(),
        params: Some(Json::Null),
    });
    let params = match next(&connection, &rx) {
        Message::Notification { method, params: Some(json) } if method == method::PUBLISH_DIAGNOSTICS => {
            PublishDiagnosticsParams::from_json(&json).expect("诊断参数能解析")
        }
        other => panic!("应是诊断推送：{other:?}"),
    };
    assert_eq!(params.uri, "file:///test/main.go");
    assert_eq!(params.diagnostics.len(), 2);
    assert!(params.diagnostics.iter().any(|d| d.severity == Severity::Error && d.message == "undefined: x"));
    assert!(params.diagnostics.iter().any(|d| d.severity == Severity::Warning && d.message == "unused variable"));

    // didOpen：诊断按文档的 uri 与版本推。
    connection.send(&Message::Notification {
        method: method::DID_OPEN.into(),
        params: Some(
            DidOpenParams {
                uri: "file:///opened/main.go".into(),
                language_id: "go".into(),
                version: 1,
                text: "package main\n".into(),
            }
            .to_json(),
        ),
    });
    match next(&connection, &rx) {
        Message::Notification { params: Some(json), .. } => {
            let params = PublishDiagnosticsParams::from_json(&json).expect("诊断参数能解析");
            assert_eq!(params.uri, "file:///opened/main.go");
            assert_eq!(params.version, Some(1));
            assert_eq!(params.diagnostics.len(), 1);
        }
        other => panic!("应是 didOpen 的诊断：{other:?}"),
    }

    // 补全：三个固定候选，列表标记不完整。
    connection.send(&Message::Request {
        id: Id::Number(2),
        method: method::COMPLETION.into(),
        params: Some(
            CompletionParams {
                uri: "file:///opened/main.go".into(),
                position: Position { line: 0, character: 0 },
                context: None,
            }
            .to_json(),
        ),
    });
    match next(&connection, &rx) {
        Message::Response { result: Some(json), .. } => {
            let completion = CompletionResponse::from_json(&json).expect("补全响应能解析");
            let labels: Vec<&str> = completion.items().iter().map(|item| item.label.as_str()).collect();
            assert_eq!(labels, ["Print", "Printf", "Println"]);
            assert!(matches!(&completion, CompletionResponse::List(list) if list.is_incomplete));
        }
        other => panic!("应是补全响应：{other:?}"),
    }

    // 关闭：shutdown 应答 null，exit 之后连接报告断开。
    connection.send(&Message::Request {
        id: Id::Number(3),
        method: method::SHUTDOWN.into(),
        params: None,
    });
    match next(&connection, &rx) {
        Message::Response { result: Some(result), .. } => assert_eq!(result, Json::Null),
        other => panic!("应是 shutdown 响应：{other:?}"),
    }
    connection.send(&Message::Notification { method: method::EXIT.into(), params: None });
    closed(&connection, &rx);
    // 断开是持久的：再取还是 Closed。
    assert!(matches!(connection.poll(), Poll::Closed));
}

#[test]
fn missing_program_fails_to_start() {
    let (tx, _rx) = channel();
    let error = Connection::start(Path::new("winc-不存在的语言服务器"), move || {
        let _ = tx.send(());
    });
    assert!(error.is_err(), "不存在的程序应报错");
}

#[test]
fn crashed_server_is_reported_as_closed() {
    let (tx, rx) = channel();
    let connection = Connection::start(Path::new(env!("CARGO_BIN_EXE_fake_crasher")), move || {
        let _ = tx.send(());
    })
    .expect("能启动");
    // 服务器立即崩溃：读线程读到结尾，连接报告断开并唤醒。
    closed(&connection, &rx);
    assert!(matches!(connection.poll(), Poll::Closed));
}
