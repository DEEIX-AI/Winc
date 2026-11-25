//! golden 报文回放：`tests/golden/` 里的样本采集自真实 gopls v0.24 会话（initialize 响应、
//! 诊断推送、补全响应）。类型或转换改动后跑这个测试，保证解析结果与真实服务器一致。
//! 样本的采集方式见变更 `add-lsp-go` 的 manual-verification.md。

use std::path::Path;

use winc_json::Json;
use winc_lsp::message::{
    CompletionResponse, PublishDiagnosticsParams, ServerCapabilities, Severity,
};

/// 读取 golden 文件并解析成 `Json`。
fn golden(name: &str) -> Json {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    let text = std::fs::read_to_string(path).expect("golden 文件存在");
    Json::parse(&text).expect("golden 报文是合法 JSON")
}

/// 从报文信封里取 `result`（响应）字段。
fn result(message: &Json) -> Json {
    message
        .as_object()
        .and_then(|map| map.get("result"))
        .cloned()
        .expect("响应有 result")
}

/// 从报文信封里取 `params`（通知）字段。
fn params(message: &Json) -> Json {
    message
        .as_object()
        .and_then(|map| map.get("params"))
        .cloned()
        .expect("通知有 params")
}

#[test]
fn gopls_initialize_result() {
    let caps = ServerCapabilities::from_json(&result(&golden("initialize-result.json")))
        .expect("gopls 的握手响应能解析");
    // gopls 用对象形式：`textDocumentSync.change = 2`（增量）。
    assert!(caps.incremental);
    assert_eq!(caps.completion_trigger_characters, ["."]);
}

#[test]
fn gopls_publish_diagnostics() {
    let message = golden("publish-diagnostics.json");
    let diagnostics = PublishDiagnosticsParams::from_json(&params(&message))
        .expect("诊断推送能解析");
    assert!(diagnostics.uri.ends_with("main.go"));
    assert_eq!(diagnostics.version, Some(1));
    // 两条编译错误：`fmt.Pri` 未定义（行 5）与 `undefinedSymbol` 未定义（行 6）。
    assert_eq!(diagnostics.diagnostics.len(), 2);
    assert!(diagnostics.diagnostics.iter().all(|d| d.severity == Severity::Error));
    assert!(diagnostics
        .diagnostics
        .iter()
        .any(|d| d.message.contains("undefined: fmt.Pri")));
    assert_eq!(diagnostics.diagnostics[0].range.start.line, 5);
    assert_eq!(diagnostics.diagnostics[0].range.start.character, 5);
}

#[test]
fn gopls_completion_response() {
    let completion =
        CompletionResponse::from_json(&result(&golden("completion-response.json")))
            .expect("补全响应能解析");
    // gopls 对 `fmt.Pri` 的补全：Print、Printf、Println，且列表标记为不完整
    // （继续输入应重新请求）。
    let labels: Vec<&str> = completion.items().iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels, ["Print", "Printf", "Println"]);
    assert!(matches!(&completion, CompletionResponse::List(list) if list.is_incomplete));
    // 真实 gopls 的候选带 `textEdit`（编辑范围覆盖已输入的前缀）。
    let first = &completion.items()[0];
    let edit = first.text_edit.as_ref().expect("gopls 候选带 textEdit");
    assert!(edit.new_text.contains("Print"));
    // kind 是函数类（3）。
    assert_eq!(first.kind, Some(3));
}
