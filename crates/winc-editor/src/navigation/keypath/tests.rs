//! 键路径的测试：JSON、YAML、TOML 的路径计算，以及从记录点继续扫描时结果与从头扫描相同。

use super::*;
use ropey::Rope;

/// 返回 `text` 中 `|` 所在位置（不计入文档内容）的键路径文本。
fn path(language: &str, text: &str) -> String {
    let pos = text.chars().position(|c| c == '|').unwrap();
    let rope = Rope::from_str(&text.replacen('|', "", 1));
    format(&key_path(&rope, language, pos))
}

#[test]
fn json_paths_count_open_objects_and_arrays() {
    let doc = r#"{
  "name": "x",
  "dependencies": {
    "serde": { "version": "1|.0" }
  },
  "files": ["a", "b", {"path": "c|"}],
  "a.b": {"x y": 1|}
}"#;
    let cases: Vec<&str> = doc.split('|').collect();
    assert_eq!(cases.len(), 4);
    let first = format!("{}|{}", cases[0], cases[1..].join(""));
    assert_eq!(path("json", &first), "dependencies.serde.version");
    let second = format!("{}{}|{}", cases[0], cases[1], cases[2..].join(""));
    assert_eq!(path("json", &second), "files[2].path");
    let third = format!("{}{}{}|{}", cases[0], cases[1], cases[2], cases[3]);
    assert_eq!(path("json", &third), "[\"a.b\"][\"x y\"]");

    // 光标位于键上：该键计入路径。注释和字符串内的括号、引号不参与计数。
    assert_eq!(path("json", "{\"a\": {\"b|\": 1}}"), "a.b");
    assert_eq!(
        path(
            "json",
            "{\n // { [\n \"s\": \"}\\\"]\", /* ] */ \"k\": [1, 2, |3]}"
        ),
        "k[2]"
    );
    // 逗号之后、下一个键之前：位于对象内，但不在任何键内。
    assert_eq!(path("json", "{\"a\": {\"b\": 1, |}}"), "a");
    assert_eq!(path("json", "|{}"), "");
}

#[test]
fn yaml_paths_follow_indentation_and_list_items() {
    let doc = "\
name: ci
jobs:
  build:
    runs-on: ubuntu
    steps:
      - uses: checkout
      - name: Test
        run: cargo test|
      - run: echo
";
    assert_eq!(path("yaml", doc), "jobs.build.steps[1].run");
    assert_eq!(path("yaml", "a:\n  b: |1\n"), "a.b");
    assert_eq!(path("yaml", "a:\n  b:\n    - x\n    - |y\n"), "a.b[1]");
    // 列表项的缩进与上级键相同。
    assert_eq!(path("yaml", "list:\n- one\n- two|\n"), "list[1]");
    // 光标在空行或注释行：按其上方最近的有内容的行计算。
    assert_eq!(path("yaml", "a:\n  b: 1\n  # 注释\n|"), "a.b");
    assert_eq!(path("yaml", "\"quoted key\": |1\n"), "[\"quoted key\"]");
    assert_eq!(path("yaml", "|"), "");
}

#[test]
fn toml_paths_use_the_table_header() {
    assert_eq!(path("toml", "[package]\nname = \"x|\"\n"), "package.name");
    assert_eq!(
        path("toml", "[[bin]]\nname = \"a\"\n[[bin]]\nname = |\"b\"\n"),
        "bin[1].name"
    );
    assert_eq!(
        path("toml", "[dependencies]\nserde.version = |\"1\"\n"),
        "dependencies.serde.version"
    );
    assert_eq!(path("toml", "top = 1|\n[a]\n"), "top");
    assert_eq!(path("toml", "[a.\"b.c\"]\n|"), "a[\"b.c\"]");
}

#[test]
fn other_languages_have_no_key_path() {
    assert_eq!(path("rust", "fn |main() {}"), "");
}

#[test]
fn resumed_json_scans_match_a_scan_from_the_start() {
    // 长度是 `CHECKPOINT` 的四倍以上，带注释、转义和没写完的字符串，记录点落在各种状态里。
    let item =
        "  {\"name\": \"a\\\"b\", /* c */ \"list\": [1, {\"k\": 2}], // d\n  \"é\": [\"x\"]},\n";
    let mut text = format!(
        "{{\"items\": [\n{}",
        item.repeat(CHECKPOINT / item.len() * 4)
    );
    let mut scan = KeyPathScan::default();
    let check = |scan: &mut KeyPathScan, rope: &Rope| {
        let n = rope.len_chars();
        for pos in (0..n).step_by(997).chain([n, n / 2, 3, n - 1]) {
            assert_eq!(scan.json(rope, pos), key_path(rope, "json", pos), "{pos}");
        }
    };
    check(&mut scan, &Rope::from_str(&text));
    // 在中间改一处：后面的记录作废，前面的照用。
    let mid = text.len() / 2;
    let mid = (mid..).find(|&i| text.is_char_boundary(i)).unwrap();
    text.insert_str(mid, "{\"new\": [");
    check(&mut scan, &Rope::from_str(&text));
    // 改成更短的文字，记录点比全文还多。
    text.truncate(100);
    check(&mut scan, &Rope::from_str(&text));
}
