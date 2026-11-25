//! 报文信封与分帧的测试：round trip、粘包、半包、坏头与额外头。

use super::*;

fn request(method: &str, id: i64, params: Json) -> Message {
    Message::Request {
        id: Id::Number(id),
        method: method.into(),
        params: Some(params),
    }
}

#[test]
fn envelope_round_trip() {
    let message = request("initialize", 1, Json::object([("processId", Json::Number(7.0))]));
    assert_eq!(Message::from_json(&message.to_json()), Some(message));

    // 通知没有 id。
    let notification = Message::Notification {
        method: "textDocument/didOpen".into(),
        params: Some(Json::Null),
    };
    assert_eq!(Message::from_json(&notification.to_json()), Some(notification.clone()));
    assert_eq!(notification.method(), Some("textDocument/didOpen"));

    // 不带参数的通知（params 字段不写）。
    let bare = Message::Notification { method: "exit".into(), params: None };
    assert_eq!(Message::from_json(&bare.to_json()), Some(bare));

    // 成功响应的 result 可以是 null（字段写了就算有结果）。
    let null_result = parse(r#"{"id": 3, "jsonrpc": "2.0", "result": null}"#);
    let message = Message::from_json(&null_result).expect("null 也是结果");
    assert!(matches!(message, Message::Response { result: Some(Json::Null), error: None, .. }));

    // 错误响应。
    let error = parse(r#"{"error": {"code": -32601, "message": "method not found"}, "id": 9, "jsonrpc": "2.0"}"#);
    let message = Message::from_json(&error).expect("错误响应");
    match message {
        Message::Response { result: None, error: Some(error), .. } => {
            assert_eq!(error.code, -32601);
            assert_eq!(error.message, "method not found");
        }
        _ => panic!("应是错误响应"),
    }

    // 字符串 id 原样保留（服务器请求可能用字符串）。
    let text_id = parse(r#"{"id": "cfg-17", "jsonrpc": "2.0", "result": []}"#);
    let message = Message::from_json(&text_id).expect("字符串 id");
    assert!(matches!(message, Message::Response { id: Id::Text(ref text), .. } if text == "cfg-17"));

    // 既没有 method 也没有 result/error 的报文按无法理解丢弃。
    assert!(Message::from_json(&parse(r#"{"id": 4, "jsonrpc": "2.0"}"#)).is_none());
    assert!(Message::from_json(&parse("123")).is_none());
}

#[test]
fn frame_encoding_round_trip() {
    let message = request("textDocument/completion", 42, Json::object([("ok", Json::Bool(true))]));
    let bytes = frame(&message);
    let text = String::from_utf8(bytes.clone()).expect("帧是 UTF-8");
    assert!(text.starts_with("Content-Length: "));
    assert!(text.ends_with("}\r\n") || text.contains("\r\n\r\n"));

    let mut decoder = Decoder::new();
    decoder.push(&bytes);
    assert_eq!(decoder.next_message(), Some(message));
    assert_eq!(decoder.next_message(), None, "只有一条");

    // 非 ASCII 体：头里的长度按字节算。
    let notification = Message::Notification {
        method: "textDocument/publishDiagnostics".into(),
        params: Some(Json::String("中文😀".into())),
    };
    let mut decoder = Decoder::new();
    decoder.push(&frame(&notification));
    assert_eq!(decoder.next_message(), Some(notification));
}

#[test]
fn decoder_splits_stuck_together_frames() {
    let first = frame(&request("initialize", 1, Json::Null));
    let second = frame(&Message::Notification { method: "exit".into(), params: None });
    // 粘包：两条帧一次喂入。
    let mut stuck = first.clone();
    stuck.extend_from_slice(&second);
    let mut decoder = Decoder::new();
    decoder.push(&stuck);
    assert_eq!(decoder.next_message(), Some(request("initialize", 1, Json::Null)));
    assert_eq!(
        decoder.next_message(),
        Some(Message::Notification { method: "exit".into(), params: None })
    );
    assert_eq!(decoder.next_message(), None);
}

#[test]
fn decoder_waits_for_half_frame() {
    let bytes = frame(&request("shutdown", 5, Json::Null));
    // 半包：一个分块一个分块喂，只有最后的分块喂入后才有消息。
    let mut decoder = Decoder::new();
    let chunks: Vec<&[u8]> = bytes.chunks(3).collect();
    for chunk in &chunks[..chunks.len() - 1] {
        decoder.push(chunk);
        assert_eq!(decoder.next_message(), None, "没喂完不该有消息");
    }
    decoder.push(chunks.last().expect("至少一个分块"));
    assert_eq!(decoder.next_message(), Some(request("shutdown", 5, Json::Null)));

    // 跨帧半包：第一条完整、第二条只有一半。
    let mut decoder = Decoder::new();
    let second = frame(&request("exit", 6, Json::Null));
    let cut = second.len() / 2;
    let mut input = frame(&request("shutdown", 5, Json::Null));
    input.extend_from_slice(&second[..cut]);
    decoder.push(&input);
    assert_eq!(decoder.next_message(), Some(request("shutdown", 5, Json::Null)));
    assert_eq!(decoder.next_message(), None, "第二条还不完整");
    decoder.push(&second[cut..]);
    assert_eq!(decoder.next_message(), Some(request("exit", 6, Json::Null)));
}

#[test]
fn decoder_skips_bad_header_and_resyncs() {
    // 坏头（没有 Content-Length）后面跟一条好帧：坏头跳过，好帧照常解出。
    let bad = b"X-Junk: yes\r\nContent-Type: garbage\r\n\r\n";
    let good = frame(&request("initialize", 2, Json::Null));
    let mut decoder = Decoder::new();
    decoder.push(bad);
    decoder.push(&good);
    assert_eq!(decoder.next_message(), Some(request("initialize", 2, Json::Null)));
}

#[test]
fn decoder_tolerates_extra_and_uppercase_headers() {
    // 头名大小写不敏感，未知头（Content-Type）忽略。
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"m"}"#;
    let raw = format!(
        "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-LENGTH: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let mut decoder = Decoder::new();
    decoder.push(raw.as_bytes());
    let message = decoder.next_message().expect("头大小写不敏感");
    assert_eq!(message.method(), Some("m"));
}

fn parse(text: &str) -> Json {
    Json::parse(text).expect("测试样本是合法 JSON")
}
