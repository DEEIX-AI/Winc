//! 格式化命令拆分的测试：按空白与引号拆参数、`~/` 展开，以及占位符展开后仍是同一个参数。

use super::*;

fn args(command: &str) -> Vec<String> {
    split(command, Some(Path::new("/home/me"))).unwrap()
}

#[test]
fn splits_on_spaces_and_respects_quotes() {
    assert_eq!(args("jq --indent 4 ."), ["jq", "--indent", "4", "."]);
    assert_eq!(args("  gofmt  "), ["gofmt"]);
    assert_eq!(
        args(r#"sh -c 'jq . | cat' "a \"b\" c" x"y"z"#),
        ["sh", "-c", "jq . | cat", r#"a "b" c"#, "xyz"]
    );
    assert_eq!(args("''"), [""]);
    assert_eq!(
        args("~/bin/fmt ~/x a~/b '~/c'"),
        ["/home/me/bin/fmt", "/home/me/x", "a~/b", "~/c"]
    );
    assert!(split("jq 'oops", None).is_err());
    assert!(split("jq \"oops", None).is_err());
}

#[test]
fn placeholders_stay_one_argument() {
    let file = Path::new("/tmp/有 空格/it's $(x).rs");
    assert_eq!(
        expand("--stdin-filepath={file}", file, None),
        OsString::from("--stdin-filepath=/tmp/有 空格/it's $(x).rs")
    );
    assert_eq!(expand("{rust-edition}", file, Some("2024")), "2024");
    assert_eq!(expand("{rust-edition}", file, None), "2021");
    assert_eq!(expand("{a: .b}", file, None), "{a: .b}");
    assert_eq!(
        expand("{file}:{file}", Path::new("a"), None),
        OsString::from("a:a")
    );
}
