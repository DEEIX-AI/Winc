//! [`Json`] 的测试：解析、嵌套深度上限、转义与序列化。

use super::*;

#[test]
fn nested_values_are_limited_in_depth() {
    // 解析是递归的：一长串 `[` 会耗尽调用栈。
    assert!(Json::parse(&"[".repeat(200)).is_none(), "没闭合的深层嵌套");
    let ok = format!("{}{}", "[".repeat(200), "]".repeat(200));
    assert!(Json::parse(&ok).is_some(), "两百层还行");
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(
        Json::parse(&deep).is_none(),
        "十万层当格式错误，不能把栈用光"
    );
}

#[test]
fn literals_and_escapes() {
    assert_eq!(
        Json::parse(" {\"a\": [1, 2.5e3, true, null, \"中\\u00e9\"]} ")
            .and_then(|v| v.as_object().cloned())
            .map(|o| o.len()),
        Some(1)
    );
    assert_eq!(
        Json::parse(r#""中文😀a\nb\"c""#),
        Some(Json::String("中文😀a\nb\"c".into()))
    );
    assert!(Json::parse("\"没有收尾").is_none());
    assert!(Json::parse("{").is_none());
    assert!(Json::parse("1 2").is_none());
    assert!(Json::parse("").is_none());
}

#[test]
fn unicode_escape_takes_exactly_four_hex_digits() {
    assert_eq!(Json::parse(r#""A""#), Some(Json::String("A".into())));
    // `from_str_radix` 接受前导 `+`，但 JSON 不允许。
    assert!(Json::parse(r#""\u+041""#).is_none());
    assert!(Json::parse(r#""\u-041""#).is_none());
    assert!(Json::parse(r#""\u00G1""#).is_none());
}

#[test]
fn jsonc_takes_comments_trailing_commas_and_a_bom() {
    let text = "\u{feff}{\n  // 字号\n  \"size\": 16, /* 以后再调 */\n  \"list\": [1, 2,],\n}\n";
    let (value, comments) = Json::parse_jsonc(text).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object["size"], Json::Number(16.0));
    assert_eq!(
        object["list"],
        Json::Array(vec![Json::Number(1.0), Json::Number(2.0)])
    );
    assert!(comments);
    assert_eq!(
        Json::parse_jsonc("{\"a\": 1,}"),
        Some((Json::object([("a", Json::Number(1.0))]), false))
    );
    // 字符串中的 `//` 不是注释；没有结束的注释、只有一个逗号的对象和数组都无法解析。
    assert_eq!(
        Json::parse_jsonc(r#""a // b""#).unwrap().0,
        Json::String("a // b".into())
    );
    assert!(Json::parse_jsonc("{} /* 没写完").is_none());
    assert!(Json::parse_jsonc("[,]").is_none());
    assert!(Json::parse_jsonc("{,}").is_none());
    // 程序写出的文件（状态、备份头）和 `package.json` 仍按严格的 JSON 解析。
    assert!(Json::parse("{\"a\": 1,}").is_none());
    assert!(Json::parse("// x\n{}").is_none());
}

#[test]
fn numbers_json_cannot_hold_are_written_as_null() {
    let value = Json::Array(vec![
        Json::Number(f64::NAN),
        Json::Number(f64::INFINITY),
        Json::Number(1.5),
    ]);
    let text = value.write(0);
    assert_eq!(
        Json::parse(&text),
        Some(Json::Array(vec![Json::Null, Json::Null, Json::Number(1.5)]))
    );
}
