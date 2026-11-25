//! [`Json`] 与 LSP 消息子集双向转换的测试。golden 报文（真实 gopls 会话样本）在
//! `tests/golden.rs` 里回放，这里测转换逻辑本身。

use super::*;

fn parse(text: &str) -> Json {
    Json::parse(text).expect("测试样本都是合法 JSON")
}

#[test]
fn position_round_trip() {
    let position = Position { line: 12, character: 7 };
    assert_eq!(Position::from_json(&position.to_json()), Some(position));
    assert_eq!(parse(r#"{"character": 3, "line": 0}"#).as_object().map(|m| m.len()), Some(2));
    // 缺字段或类型不对都返回 `None`。
    assert!(Position::from_json(&parse(r#"{"line": 1}"#)).is_none());
    assert!(Position::from_json(&parse(r#"{"line": "x", "character": 0}"#)).is_none());
}

#[test]
fn range_round_trip() {
    let range = Range {
        start: Position { line: 1, character: 0 },
        end: Position { line: 1, character: 4 },
    };
    assert_eq!(Range::from_json(&range.to_json()), Some(range));
}

#[test]
fn text_edit_round_trip() {
    let edit = TextEdit {
        range: Range {
            start: Position { line: 0, character: 2 },
            end: Position { line: 0, character: 5 },
        },
        new_text: "Println".into(),
    };
    assert_eq!(TextEdit::from_json(&edit.to_json()), Some(edit.clone()));
    let json = parse(r#"{"newText": "你好", "range": {"end": {"character": 1, "line": 0}, "start": {"character": 0, "line": 0}}}"#);
    assert_eq!(TextEdit::from_json(&json).map(|edit| edit.new_text), Some("你好".into()));
}

#[test]
fn diagnostic_severity_and_default() {
    let error = Diagnostic::from_json(&parse(
        r#"{"message": "undefined: foo", "range": {"end": {"character": 8, "line": 3}, "start": {"character": 5, "line": 3}}, "severity": 1}"#,
    ))
    .expect("完整诊断");
    assert_eq!(error.severity, Severity::Error);
    assert_eq!(error.message, "undefined: foo");

    // severity 缺失按 LSP 规定视为错误；3、4（信息、提示）按警告。
    let no_severity = Diagnostic::from_json(&parse(
        r#"{"message": "m", "range": {"end": {"character": 1, "line": 0}, "start": {"character": 0, "line": 0}}}"#,
    ))
    .expect("severity 可选");
    assert_eq!(no_severity.severity, Severity::Error);
    let hint = Diagnostic::from_json(&parse(
        r#"{"message": "m", "range": {"end": {"character": 1, "line": 0}, "start": {"character": 0, "line": 0}}, "severity": 4}"#,
    ))
    .expect("hint 也收");
    assert_eq!(hint.severity, Severity::Warning);

    // 自己写出的诊断能读回来。
    assert_eq!(
        Diagnostic::from_json(&error.to_json()),
        Some(error),
        "round trip"
    );
}

#[test]
fn completion_item_fields() {
    let json = parse(
        r#"{"additionalTextEdits": [{"newText": "\t\"strings\"", "range": {"end": {"character": 0, "line": 3}, "start": {"character": 0, "line": 3}}}], "filterText": "Print", "insertText": "Println", "kind": 3, "label": "Println", "sortText": "00001", "textEdit": {"newText": "Println", "range": {"end": {"character": 9, "line": 6}, "start": {"character": 5, "line": 6}}}}"#,
    );
    let item = CompletionItem::from_json(&json).expect("候选字段齐全");
    assert_eq!(item.label, "Println");
    assert_eq!(item.kind, Some(3));
    assert_eq!(item.insert_text.as_deref(), Some("Println"));
    assert!(item.text_edit.is_some());
    assert_eq!(item.additional_text_edits.len(), 1);

    // 只剩 label 也行（kind、编辑等全可选）。
    let bare = CompletionItem::from_json(&parse(r#"{"label": "foo"}"#)).expect("label 必有");
    assert_eq!(bare.label, "foo");
    assert_eq!(bare.kind, None);
    assert!(bare.additional_text_edits.is_empty());
    assert!(CompletionItem::from_json(&parse(r#"{"kind": 1}"#)).is_none());
}

#[test]
fn completion_response_both_forms() {
    let list = CompletionResponse::from_json(&parse(
        r#"{"isIncomplete": false, "items": [{"label": "a"}, {"label": "b"}]}"#,
    ))
    .expect("列表形式");
    assert_eq!(list.items().len(), 2);
    assert!(matches!(&list, CompletionResponse::List(list) if !list.is_incomplete));

    let items = CompletionResponse::from_json(&parse(r#"[{"label": "a"}]"#)).expect("数组形式");
    assert_eq!(items.items().len(), 1);
    assert!(matches!(items, CompletionResponse::Items(_)));

    // 两种形式之外（例如 null）按无法理解丢弃。
    assert!(CompletionResponse::from_json(&parse("null")).is_none());
}

#[test]
fn initialize_serialization() {
    let params = InitializeParams {
        process_id: 1234,
        root_uri: Some("file:///D:/code/go".into()),
    };
    let json = params.to_json();
    let map = json.as_object().expect("对象");
    assert_eq!(map.get("processId").and_then(Json::as_number), Some(1234.0));
    assert_eq!(
        map.get("rootUri").and_then(Json::as_str),
        Some("file:///D:/code/go")
    );
    // 客户端能力里声明了补全。
    let caps = map.get("capabilities").and_then(Json::as_object).expect("capabilities");
    assert!(caps.contains_key("textDocument"));

    // 无根（散开的单文件）时 rootUri 为 null。
    let no_root = InitializeParams { process_id: 1, root_uri: None }.to_json();
    assert_eq!(
        no_root.as_object().and_then(|m| m.get("rootUri")),
        Some(&Json::Null)
    );
}

#[test]
fn server_capabilities_both_sync_forms() {
    // 对象形式（gopls 的写法）。
    let gopls = parse(
        r#"{"capabilities": {"completionProvider": {"triggerCharacters": ["."]}, "textDocumentSync": {"change": 2, "openClose": true}}}"#,
    );
    let caps = ServerCapabilities::from_json(&gopls).expect("gopls 形式");
    assert!(caps.incremental);
    assert_eq!(caps.completion_trigger_characters, ["."]);

    // 数字形式（其他服务器）与缺失。
    let number_form = parse(r#"{"capabilities": {"textDocumentSync": 1}}"#);
    assert!(!ServerCapabilities::from_json(&number_form).expect("数字形式").incremental);
    let empty = parse(r#"{"capabilities": {}}"#);
    let caps = ServerCapabilities::from_json(&empty).expect("能力可缺");
    assert!(!caps.incremental);
    assert!(caps.completion_trigger_characters.is_empty());
}

#[test]
fn document_sync_serialization() {
    let open = DidOpenParams {
        uri: "file:///home/x/main.go".into(),
        language_id: "go".into(),
        version: 1,
        text: "package main\n".into(),
    };
    let json = open.to_json();
    let doc = json
        .as_object()
        .and_then(|m| m.get("textDocument"))
        .and_then(Json::as_object)
        .expect("textDocument");
    assert_eq!(doc.get("languageId").and_then(Json::as_str), Some("go"));
    assert_eq!(doc.get("version").and_then(Json::as_number), Some(1.0));

    let change = DidChangeParams {
        uri: "file:///home/x/main.go".into(),
        version: 2,
        changes: vec![
            ContentChange::Incremental {
                range: Range {
                    start: Position { line: 0, character: 12 },
                    end: Position { line: 0, character: 12 },
                },
                text: "\n".into(),
            },
            ContentChange::Full("package main\n\n".into()),
        ],
    };
    let json = change.to_json();
    let changes = json
        .as_object()
        .and_then(|m| m.get("contentChanges"))
        .and_then(Json::as_array)
        .expect("contentChanges");
    assert_eq!(changes.len(), 2);
    assert!(changes[0].as_object().expect("增量带范围").contains_key("range"));
    assert!(!changes[1].as_object().expect("全量不带范围").contains_key("range"));

    let close = DidCloseParams { uri: "file:///home/x/main.go".into() }.to_json();
    assert_eq!(
        close
            .as_object()
            .and_then(|m| m.get("textDocument"))
            .and_then(Json::as_object)
            .and_then(|doc| doc.get("uri"))
            .and_then(Json::as_str),
        Some("file:///home/x/main.go")
    );
}

#[test]
fn completion_params_with_and_without_context() {
    let params = CompletionParams {
        uri: "file:///home/x/main.go".into(),
        position: Position { line: 5, character: 7 },
        context: Some(CompletionContext {
            trigger_kind: 2,
            trigger_character: Some(".".into()),
        }),
    };
    let json = params.to_json();
    let map = json.as_object().expect("对象");
    assert!(map.contains_key("position"));
    assert!(map.contains_key("context"));
    assert!(map.contains_key("textDocument"));

    let manual = CompletionParams {
        uri: "file:///home/x/main.go".into(),
        position: Position { line: 5, character: 0 },
        context: None,
    };
    assert!(!manual.to_json().as_object().expect("对象").contains_key("context"));
}

#[test]
fn publish_diagnostics_params() {
    let json = parse(
        r#"{"diagnostics": [{"message": "undefined: foo", "range": {"end": {"character": 10, "line": 5}, "start": {"character": 7, "line": 5}}, "severity": 1}], "uri": "file:///home/x/main.go", "version": 3}"#,
    );
    let params = PublishDiagnosticsParams::from_json(&json).expect("诊断推送");
    assert_eq!(params.uri, "file:///home/x/main.go");
    assert_eq!(params.version, Some(3));
    assert_eq!(params.diagnostics.len(), 1);
    assert_eq!(params.diagnostics[0].severity, Severity::Error);

    // version 可缺失。
    let no_version = parse(r#"{"diagnostics": [], "uri": "file:///x"}"#);
    let params = PublishDiagnosticsParams::from_json(&no_version).expect("version 可缺");
    assert_eq!(params.version, None);
}

#[test]
fn show_message_params() {
    let json = parse(r#"{"message": "gopls requires a module", "type": 1}"#);
    let params = ShowMessageParams::from_json(&json).expect("消息");
    assert_eq!(params.message, "gopls requires a module");
}

#[cfg(unix)]
#[test]
fn file_uri_unix() {
    assert_eq!(file_uri(Path::new("/home/x/main.go")).as_deref(), Some("file:///home/x/main.go"));
    assert_eq!(file_uri(Path::new("/tmp/a b.go")).as_deref(), Some("file:///tmp/a%20b.go"));
    assert_eq!(file_uri(Path::new("/代码/主.go")).as_deref(), Some("file:///%E4%BB%A3%E7%A0%81/%E4%B8%BB.go"));
    // 相对路径与空路径没有根。
    assert_eq!(file_uri(Path::new("main.go")), None);
    assert_eq!(file_uri(Path::new("")), None);
}

#[cfg(windows)]
#[test]
fn file_uri_windows() {
    assert_eq!(file_uri(Path::new(r"D:\code\main.go")).as_deref(), Some("file:///D:/code/main.go"));
    assert_eq!(file_uri(Path::new("D:/code/a b.go")).as_deref(), Some("file:///D:/code/a%20b.go"));
    // Unix 风格根也能转（挂在当前盘符下的场景不出现，但编码路径一致）。
    assert_eq!(file_uri(Path::new("/code/main.go")).as_deref(), Some("file:///code/main.go"));
}

#[test]
fn uri_to_path_round_trip() {
    for (path, uri) in [
        (r"D:\code\main.go", "file:///D:/code/main.go"),
        ("D:/code/a b.go", "file:///D:/code/a%20b.go"),
        ("/home/x/main.go", "file:///home/x/main.go"),
        ("/代码/主.go", "file:///%E4%BB%A3%E7%A0%81/%E4%B8%BB.go"),
    ] {
        // 输入按 `/` 分隔（file_uri 的输出形态），来回一致。
        let forward_slashes = path.replace('\\', "/");
        assert_eq!(uri_to_path(uri).as_deref(), Some(Path::new(&forward_slashes)), "{uri}");
    }
    // 省掉第三个斜杠的 Windows 写法。
    assert_eq!(uri_to_path("file://D:/code/x.go").as_deref(), Some(Path::new("D:/code/x.go")));
    // 非 file 协议、带主机名、坏编码都拒绝。
    assert_eq!(uri_to_path("http://example.com/x"), None);
    assert_eq!(uri_to_path("file://server/share/x.go"), None);
    assert_eq!(uri_to_path("file:///D%/x"), None);
}
