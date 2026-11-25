//! Nix 与 HCL 字符串的词法测试：缩进字符串的转义、`$$`、跨行的双引号字符串和带 `'` 的名字。

use super::*;

/// 缩进字符串中内嵌 shell 脚本：`''${` 是转义，其后的 `/*` 仍在字符串中，`'';` 后恢复为代码。
#[test]
fn nix_indented_strings_keep_escapes_inside() {
    let sample = [
        "let",
        "  s = ''",
        "    x=\"''${FOO}\"",
        "    \"$dir\"/*)",
        "  '';",
        "in s",
    ];
    let mut state = LineState::Normal;
    let mut lines = Vec::new();
    for line in sample {
        let (tokens, next) = lex("a.nix", line, state);
        lines.push(tokens);
        state = next;
    }
    assert_eq!(
        lines[2],
        [
            ("    x=\"".into(), TokenKind::String),
            ("''$".into(), TokenKind::Escape),
            ("{FOO}\"".into(), TokenKind::String),
        ]
    );
    assert_eq!(lines[3], [("    \"$dir\"/*)".into(), TokenKind::String)]);
    assert_eq!(lines[5], [("in".into(), TokenKind::Keyword)]);
    assert_eq!(state, LineState::Normal);

    let inside = LineState::BlockString(0);
    let escapes = texts("a.nix", r"'''a''' ''\n ''\'", inside, TokenKind::Escape);
    assert_eq!(escapes, ["'''", "'''", r"''\n", r"''\'"]);
    let (_, next) = lex("a.nix", r"'''a''' ''\n ''\'", inside);
    assert_eq!(next, inside);
}

/// `$$` 是字面字符：其后的 `{` 不开始插值；缩进字符串中的 `''$` 先作为转义，其后的 `${` 仍是
/// 插值。HCL 的 `$${` 同样是字面的 `${`。
#[test]
fn doubled_dollars_do_not_interpolate() {
    let strings = |file: &str, line: &str| texts(file, line, LineState::Normal, TokenKind::String);
    assert_eq!(
        strings("a.nix", r#"a = "$${x}" + ''$${y}'';"#),
        ["\"$${x}\"", "''$${y}''"]
    );
    assert_eq!(strings("a.nix", "b = ''''$${x}'';"), ["''", "${", "}''"]);
    assert_eq!(
        strings("main.tf", r#"a = "$${x} ${y}""#),
        ["\"$${x} ${", "}\""]
    );
}

/// Nix 的双引号字符串可以跨行；名字中的 `'` 不开始字符串（`x''` 是一个名字）。
#[test]
fn nix_strings_span_lines_and_names_take_primes() {
    let (_, next) = lex("a.nix", r#"c = "line1"#, LineState::Normal);
    assert_eq!(next, LineState::DoubleQuote);
    let (tokens, next) = lex("a.nix", r#"line2"; d = true;"#, next);
    assert_eq!(
        tokens,
        [
            ("line2\"".into(), TokenKind::String),
            ("true".into(), TokenKind::Builtin),
        ]
    );
    assert_eq!(next, LineState::Normal);

    let (tokens, next) = lex("a.nix", "f = x: x'' + inputs'.foo;", LineState::Normal);
    assert!(
        tokens.iter().all(|(_, kind)| *kind != TokenKind::String),
        "{tokens:?}"
    );
    assert_eq!(next, LineState::Normal);
}
