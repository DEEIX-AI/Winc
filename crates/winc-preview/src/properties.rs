//! Java `.properties` 的可读显示：把 `\uXXXX` 转义换成字符，其余内容原样保留。
//!
//! 只做显示用的文本变换，不解析键和值：注释、空行、重复的键、缩进和行布局都留在原来的位置，
//! 每一行单独处理，因而不会改变原文的行数和顺序。转换只认 `\u` 加四位十六进制；`\t`、`\n`
//! 等 Java 的其他转义、落单的反斜杠、无效或残缺的转义都按原文显示。

/// 把 `text` 中每一行的 Unicode 转义换成对应的字符，返回显示用的文本。
///
/// 换行符原样保留，转义不会跨行组合。成对的高、低代理项转义合成一个补充平面的字符；落单的
/// 代理项、控制字符和行分隔符保留转义写法，在屏幕上不会变成看不见的字符或断行。
pub fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('\\') {
        // 普通文字按片复制，逐个检查以反斜杠开头的片段。
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix(r"\\") {
            out.push_str(r"\\");
            rest = after;
        } else if let Some((character, after)) = decoded_escape(rest) {
            out.push(character);
            rest = after;
        } else {
            out.push('\\');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// 读取小写 u 和四位十六进制组成的 UTF-16 码元。
fn escape_unit(text: &str) -> Option<(u32, &str)> {
    let (digits, rest) = text.strip_prefix(r"\u")?.split_at_checked(4)?;
    let mut value = 0;
    for digit in digits.chars() {
        value = value * 16 + digit.to_digit(16)?;
    }
    Some((value, rest))
}

/// 相邻的高、低代理项合成一个字符。无法显示的转义留给调用方按原文复制。
fn decoded_escape(text: &str) -> Option<(char, &str)> {
    let (unit, rest) = escape_unit(text)?;
    let (scalar, rest) = if (0xD800..=0xDBFF).contains(&unit) {
        let (low, rest) = escape_unit(rest)?;
        if !(0xDC00..=0xDFFF).contains(&low) {
            return None;
        }
        (0x1_0000 + ((unit - 0xD800) << 10) + (low - 0xDC00), rest)
    } else {
        (unit, rest)
    };
    let character = char::from_u32(scalar)?;
    let preserve = character.is_control() || matches!(character, '\u{2028}' | '\u{2029}');
    (!preserve).then_some((character, rest))
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_unicode_escapes() {
        assert_eq!(decode(r"greeting=\u4F60\u597D"), "greeting=你好");
        // 已经可读的字符和转义混排时都保留。
        assert_eq!(decode(r"a=\u4F60好"), "a=你好");
        assert_eq!(decode(r"key\ u=value"), r"key\ u=value");
        // 十六进制大小写都认。
        assert_eq!(decode(r"\u4f60\u4F60"), "你你");
    }

    #[test]
    fn combines_surrogate_pairs() {
        assert_eq!(decode(r"emoji=\uD83D\uDE00"), "emoji=😀");
        // 落单的高、低代理项仍是原来的转义。
        assert_eq!(decode(r"\uD83D"), r"\uD83D");
        assert_eq!(decode(r"\uDE00"), r"\uDE00");
        assert_eq!(decode(r"\uD83Dx"), r"\uD83Dx");
        // 高代理项后面跟着的不是低代理项时，两个转义分别处理。
        assert_eq!(decode(r"\uD83DA"), "\\uD83DA");
        let separated = "\\uD83D\r\n\\uDE00";
        assert_eq!(decode(separated), separated);
    }

    #[test]
    fn keeps_literal_backslashes_and_other_escapes() {
        // 两个反斜杠是一对：后面的 `u` 不是转义。
        assert_eq!(decode(r"a=\\u4F60"), r"a=\\u4F60");
        assert_eq!(decode(r"a=\\\u4F60"), r"a=\\你");
        assert_eq!(decode(r"a=\\"), r"a=\\");
        // 标准 Java 转义不属于本变换。
        assert_eq!(decode(r"a=1\tb\:c\nd"), r"a=1\tb\:c\nd");
        // 末尾落单的反斜杠。
        assert_eq!(decode("a=\\"), "a=\\");
    }

    #[test]
    fn keeps_malformed_escapes_as_written() {
        assert_eq!(decode(r"\u12G4"), r"\u12G4");
        assert_eq!(decode(r"\u+123"), r"\u+123");
        assert_eq!(decode(r"\u你0"), r"\u你0");
        assert_eq!(decode(r"\u12"), r"\u12");
        assert_eq!(decode(r"\u"), r"\u");
        assert_eq!(decode(r"\uu0041"), r"\uu0041");
        assert_eq!(decode(r"\U4F60"), r"\U4F60");
    }

    #[test]
    fn keeps_controls_and_line_separators_escaped() {
        assert_eq!(decode(r"a=\u000A b"), r"a=\u000A b");
        assert_eq!(decode(r"a=\u0009b"), r"a=\u0009b");
        assert_eq!(decode(r"a=\u007F"), r"a=\u007F");
        assert_eq!(decode(r"a=\u2028b"), r"a=\u2028b");
        assert_eq!(decode(r"a=\u2029b"), r"a=\u2029b");
    }

    #[test]
    fn preserves_the_line_layout() {
        let source = "# 标题=\\u4F60\n\n  key\\ value = \\u597D\\\nnext=\\u4E2D\r\n# 注释 \\u597D\nkey=\\u4E00\nkey=\\u4E8C\n";
        let shown = decode(source);
        assert_eq!(
            shown,
            "# 标题=你\n\n  key\\ value = 好\\\nnext=中\r\n# 注释 好\nkey=一\nkey=二\n"
        );
        assert_eq!(shown.lines().count(), source.lines().count());
    }
}
