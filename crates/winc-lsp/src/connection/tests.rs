//! 连接的纯逻辑部分（自动应答）的测试；进程级行为在 `tests/lifecycle.rs` 里用假服务器覆盖。

use super::*;

fn params(text: &str) -> Option<Json> {
    Json::parse(text)
}

#[test]
fn configuration_request_gets_null_array_matching_items() {
    let json = params(
        r#"{"items": [{"section": "gopls"}, {"section": "gopls"}], "scopeUri": "file:///x"}"#,
    );
    let reply = auto_reply_result("workspace/configuration", &json);
    let items = reply.as_array().expect("数组结果");
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|item| *item == Json::Null));

    // items 缺失时回空数组。
    let empty = auto_reply_result("workspace/configuration", &params("{}"));
    assert_eq!(empty.as_array().map(<[Json]>::len), Some(0));
}

#[test]
fn other_server_requests_get_null() {
    for method in [
        "client/registerCapability",
        "window/workDoneProgress/create",
        "workspace/workspaceFolders",
    ] {
        let reply = auto_reply_result(method, &params(r#"{"registrations": []}"#));
        assert_eq!(reply, Json::Null, "{method}");
    }
    // 参数缺席也一样。
    assert_eq!(auto_reply_result("client/registerCapability", &None), Json::Null);
}
