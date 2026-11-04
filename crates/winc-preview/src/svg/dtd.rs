//! DOCTYPE 内部子集：只展开值为纯文本的实体，去掉 DOCTYPE 后再交给不支持 DTD 的 XML 解析。
//!
//! SVG 导出时常把命名空间和样式写成实体（`<!ENTITY ns_extend "http://…">`），必须支持。
//! `roxmltree` 自己展开实体时允许实体中带标签、实体再引用实体：数十 KB 的文件可以展开出
//! 数千层 `<g>`（绕过 [`super::nesting_exceeds`]，导致栈溢出）或数 GB 的文本，耗时随引用数
//! 按平方增长。
//!
//! 本模块只展开值中没有 `<`、`&`、`%` 的实体，展开一遍，总量有上限。

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

/// 实体展开出的文本的固定上限（字节）：取该值与原文长度乘以 [`MAX_AMPLIFICATION`] 中较大的一个。
///
/// 依据 libxml2 的实体展开限制 `XML_PARSER_ALLOWED_EXPANSION`。
const MAX_EXPANSION: usize = 1_000_000;
/// 展开出的文本相对原文长度的倍数上限（倍）。
///
/// 依据 libxml2 的 `XML_MAX_AMPLIFICATION_DEFAULT`。
const MAX_AMPLIFICATION: usize = 5;

/// XML 预定义的实体：DTD 不能重新定义这些实体。
const PREDEFINED: &[&str] = &["lt", "gt", "amp", "quot", "apos"];

/// 开头的 DOCTYPE。
struct Doctype<'a> {
    /// 在原文中的范围，包括结尾的 `>`。
    range: Range<usize>,
    /// 声明的纯文本实体：名字到值的映射；同名实体以第一次声明为准。
    entities: HashMap<&'a str, &'a str>,
}

/// 去掉 `text` 开头的 DOCTYPE，展开其中声明的纯文本实体。
///
/// 没有 DOCTYPE 时原样返回；DOCTYPE 格式错误、无法识别时同样原样返回，由 XML 解析报错。
///
/// # Errors
///
/// 展开出的文本超过 [`MAX_EXPANSION`] 等上限时返回原因。
pub(super) fn expand(text: &str) -> Result<Cow<'_, str>, String> {
    let Some(doctype) = doctype(text) else {
        return Ok(Cow::Borrowed(text));
    };
    let limit = MAX_EXPANSION.max(text.len().saturating_mul(MAX_AMPLIFICATION));
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..doctype.range.start]);
    // DOCTYPE 替换为相同数量的换行，使 XML 错误报告的行号与原文一致。
    let lines = text[doctype.range.clone()].matches('\n').count();
    out.extend(std::iter::repeat_n('\n', lines));
    let mut expanded = 0;
    let mut rest = &text[doctype.range.end..];
    while let Some(at) = memchr::memchr2(b'&', b'<', rest.as_bytes()) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // 注释、CDATA、处理指令中的 `&` 不是实体引用，原样保留。
        let skipped = [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")]
            .into_iter()
            .find(|(open, _)| rest.starts_with(open));
        if let Some((open, close)) = skipped {
            let end = rest[open.len()..]
                .find(close)
                .map_or(rest.len(), |i| open.len() + i + close.len());
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        let Some((value, len)) = reference(rest, &doctype.entities) else {
            out.push_str(&rest[..1]);
            rest = &rest[1..];
            continue;
        };
        expanded += value.len();
        if expanded > limit {
            return Err(winc_i18n::t!("preview-svg-entities-too-large").to_owned());
        }
        // 值中的引号写成字符引用，展开在属性值中时不会提前结束该属性。
        for c in value.chars() {
            match c {
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                c => out.push(c),
            }
        }
        rest = &rest[len..];
    }
    out.push_str(rest);
    Ok(Cow::Owned(out))
}

/// 解析 `text` 开头的实体引用（`&名字;`）；实体已声明时返回其值和引用的长度（字节）。
fn reference<'a>(text: &str, entities: &HashMap<&str, &'a str>) -> Option<(&'a str, usize)> {
    let name = text.strip_prefix('&')?;
    let len = name
        .find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
        .unwrap_or(name.len());
    if !name[len..].starts_with(';') {
        return None;
    }
    let value = entities.get(&name[..len])?;
    Some((value, len + 2))
}

/// 查找开头的 DOCTYPE；之前可以有 XML 声明、注释、处理指令和空白。
fn doctype(text: &str) -> Option<Doctype<'_>> {
    let bytes = text.as_bytes();
    let mut i = if text.starts_with('\u{feff}') { 3 } else { 0 };
    loop {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let rest = &bytes[i..];
        if rest.starts_with(b"<?") {
            i = after(bytes, i, b"?>")?;
        } else if rest.starts_with(b"<!--") {
            i = after(bytes, i, b"-->")?;
        } else if rest.starts_with(b"<!DOCTYPE") {
            break;
        } else {
            return None;
        }
    }
    let start = i;
    let mut entities = HashMap::new();
    // 名字和外部标识符：引号中的 `[`、`>` 不作为分隔符。
    let mut quote = None;
    while let Some(&c) = bytes.get(i) {
        match (quote, c) {
            (None, b'"' | b'\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, b'[') => {
                i = subset(text, i + 1, &mut entities)?;
                continue;
            }
            (None, b'>') => {
                return Some(Doctype {
                    range: start..i + 1,
                    entities,
                });
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// 读取内部子集：从 `[` 之后的位置 `i` 读到 `]`，返回 `]` 之后的位置（字节偏移）。
fn subset<'a>(
    text: &'a str,
    mut i: usize,
    entities: &mut HashMap<&'a str, &'a str>,
) -> Option<usize> {
    let bytes = text.as_bytes();
    loop {
        let rest = bytes.get(i..).filter(|r| !r.is_empty())?;
        if rest.starts_with(b"]") {
            return Some(i + 1);
        } else if rest.starts_with(b"<!--") {
            i = after(bytes, i, b"-->")?;
        } else if rest.starts_with(b"<?") {
            i = after(bytes, i, b"?>")?;
        } else if rest.starts_with(b"<!") {
            let end = declaration_end(bytes, i)?;
            if let Some((name, value)) = entity(&text[i..end]) {
                entities.entry(name).or_insert(value);
            }
            i = end + 1;
        } else {
            // 空白和参数实体的引用（`%name;`）。
            i += 1;
        }
    }
}

/// 解析 `<!ENTITY 名字 "值"`（不含结尾的 `>`）；值为纯文本的内部实体时返回名字和值。
fn entity(declaration: &str) -> Option<(&str, &str)> {
    let rest = declaration.strip_prefix("<!ENTITY")?;
    if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    // 参数实体（`<!ENTITY % 名字 …>`）的第一个词是 `%`，不支持。
    let (name, rest) = rest
        .trim_start()
        .split_once(|c: char| c.is_ascii_whitespace())?;
    let rest = rest.trim_start();
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let (value, tail) = rest[1..].split_once(quote)?;
    let plain = !value.contains(['<', '&', '%']);
    (name != "%" && !PREDEFINED.contains(&name) && plain && tail.trim().is_empty())
        .then_some((name, value))
}

/// 从 `from` 起查找 `end`，返回其后的位置（字节偏移）。
fn after(bytes: &[u8], from: usize, end: &[u8]) -> Option<usize> {
    memchr::memmem::find(&bytes[from..], end).map(|i| from + i + end.len())
}

/// 从 `from`（`<!`）起查找不在引号中的第一个 `>`，返回其位置（字节偏移）。
fn declaration_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut quote = None;
    for (i, &c) in bytes.iter().enumerate().skip(from) {
        match (quote, c) {
            (None, b'"' | b'\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, b'>') => return Some(i),
            _ => {}
        }
    }
    None
}
