//! 可编辑的预览中正文（段落、标题）的回车：另起一段，列表新增一项，引用中另起一段；
//! 在空的列表项、空的引用段落上按回车时退出一层。⇧ 回车在段内换行，继续同一项或同一段引用。
//!
//! `backspace` 在光标紧跟列表记号、引用的 `>` 或标题的 `#` 时移除该标记；`shift` 处理 Tab、
//! ⇧Tab 的列表缩进与退出；`fence` 在围栏行末尾补全闭围栏。只读光标所在行及其上方若干行，
//! 不依赖解析结果，因此输入快于后台解析时结果仍正确。

/// 往上找上一层列表项时最多读取的行数（行，经验值）。
///
/// 列表项之间隔着很多空行时不再向上找，避免每次回车都扫描到文档开头。
const LOOKBACK: usize = 200;

/// 按回车要做的修改：把光标之前 `before` 个字节到光标之后 `after` 个字节替换成 `text`，光标
/// 放在 `text` 之后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// 换掉的字节数（光标之前）。
    pub before: usize,
    /// 换掉的字节数（光标之后）。
    pub after: usize,
    /// 换成的文字（换行是 `\n`，插入时按文档的行尾风格转换）。
    pub text: String,
}

impl Edit {
    fn insert(text: String) -> Self {
        Self {
            before: 0,
            after: 0,
            text,
        }
    }
}

/// 一行开头的块标记（引用、缩进、列表记号、标记之后的内容）。
#[derive(Debug, Default)]
struct Prefix<'a> {
    /// 引用的 `>`（连同前后的空格），可以有几层。
    quote: &'a str,
    /// 列表项的缩进（引用之后）。
    indent: &'a str,
    /// 列表项的记号。
    marker: Option<Marker<'a>>,
    /// 标记之后的内容。
    rest: &'a str,
}

/// 列表项的记号：`-`、`*`、`+`，或者 `1.`、`1)`；后面的空格；是否为任务列表。
#[derive(Debug)]
struct Marker<'a> {
    bullet: &'a str,
    gap: &'a str,
    task: bool,
}

impl Marker<'_> {
    /// 返回下一项的记号：符号不变，序号加一，任务框为未勾选状态。
    fn next(&self) -> String {
        let bullet = match self.bullet.split_at(self.bullet.len() - 1) {
            (digits, delim) if !digits.is_empty() => match digits.parse::<u64>() {
                Ok(n) => format!("{}{delim}", n + 1),
                Err(_) => self.bullet.to_owned(),
            },
            _ => self.bullet.to_owned(),
        };
        let task = if self.task { "[ ] " } else { "" };
        format!("{bullet}{}{task}", self.gap)
    }

    /// 记号（连同空格、任务框）占的宽度（字节）：同一项的续行缩进到这里。
    fn width(&self) -> usize {
        self.bullet.len() + self.gap.len() + if self.task { 4 } else { 0 }
    }
}

/// 解析一行开头的块标记；不是列表项时 `marker` 为 `None`，`rest` 是第一个非缩进字符之后的
/// 全部内容。
fn parse(line: &str) -> Prefix<'_> {
    let bytes = line.as_bytes();
    let mut at = 0;
    // 引用：每层前面最多 3 个空格，`>` 后面的一个空格算在标记里。
    loop {
        let spaces = bytes[at..].iter().take_while(|&&b| b == b' ').count();
        if spaces > 3 || bytes.get(at + spaces) != Some(&b'>') {
            break;
        }
        at += spaces + 1;
        if matches!(bytes.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
    }
    let quote = &line[..at];
    let indent_len = bytes[at..]
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    let indent = &line[at..at + indent_len];
    let body = &line[at + indent_len..];
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let bullet_len = match body.as_bytes().first() {
        Some(b'-' | b'*' | b'+') => 1,
        Some(b'0'..=b'9')
            if digits <= 9 && matches!(body.as_bytes().get(digits), Some(b'.' | b')')) =>
        {
            digits + 1
        }
        _ => 0,
    };
    let gap_len = body.as_bytes()[bullet_len..]
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    if bullet_len == 0 || gap_len == 0 {
        return Prefix {
            quote,
            rest: &line[at..],
            ..Default::default()
        };
    }
    let after = &body[bullet_len + gap_len..];
    let task = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .any(|t| after.starts_with(t))
        || ["[ ]", "[x]", "[X]"].contains(&after);
    let rest = if task {
        after.get(4..).unwrap_or("")
    } else {
        after
    };
    Prefix {
        quote,
        indent,
        marker: Some(Marker {
            bullet: &body[..bullet_len],
            gap: &body[bullet_len..bullet_len + gap_len],
            task,
        }),
        rest,
    }
}

/// 返回引用的层数。
fn depth(quote: &str) -> usize {
    quote.bytes().filter(|&b| b == b'>').count()
}

/// 返回去掉最内层之后的引用标记。
fn outer(quote: &str) -> &str {
    quote
        .rfind('>')
        .map_or("", |i| &quote[..i])
        .trim_end_matches(' ')
        .trim_end_matches('\t')
}

/// 返回在正文里按回车要做的修改。
///
/// `before` 是光标所在的行从行首到光标的文字，`after` 是光标到行尾（不含换行符）的文字，
/// `next` 是下一行（没有时为 `None`），`above` 从上一行起往上的各行。
pub fn enter<'a>(
    before: &str,
    after: &str,
    next: Option<&str>,
    above: impl Iterator<Item = &'a str>,
) -> Edit {
    let p = parse(before);
    let empty = p.rest.trim().is_empty() && after.trim().is_empty();
    match &p.marker {
        // 空的嵌套项：退到上一层，记号延续上一层的编号。
        Some(m) if empty && !p.indent.is_empty() => {
            let parent = above.take(LOOKBACK).map(parse).find(|q| {
                q.marker.is_some()
                    && depth(q.quote) == depth(p.quote)
                    && q.indent.len() < p.indent.len()
            });
            let text = match parent {
                Some(q) => format!(
                    "{}{}{}",
                    p.quote,
                    q.indent,
                    q.marker.as_ref().map_or_else(|| m.next(), Marker::next)
                ),
                None => format!("{}{}", p.quote, m.next()),
            };
            Edit {
                before: before.len(),
                after: after.len(),
                text,
            }
        }
        // 空的项：结束列表，另起一段。
        Some(_) if empty => Edit {
            before: before.len(),
            after: after.len(),
            text: format!("{}\n{}", p.quote.trim_end(), p.quote),
        },
        Some(m) => Edit::insert(format!("\n{}{}{}", p.quote, p.indent, m.next())),
        // 空的引用段落：退出一层引用。
        None if !p.quote.is_empty() && empty => {
            let outer = outer(p.quote);
            let inner = if outer.is_empty() {
                String::new()
            } else {
                format!("{outer} ")
            };
            Edit {
                before: before.len(),
                after: after.len(),
                text: format!("{outer}\n{inner}"),
            }
        }
        None if !p.quote.is_empty() => {
            Edit::insert(format!("\n{}\n{}", p.quote.trim_end(), p.quote))
        }
        // 空行上：只多一行。
        None if empty => Edit::insert("\n".to_owned()),
        // 段落、标题：另起一段。光标在行尾、下一行仍属于同一段时，把这一行和下一行之间的换行
        // 也替换掉，下一行成为新的一段，光标落在它的开头。
        None => {
            let joined = after.is_empty() && next.is_some_and(|l| !l.trim().is_empty());
            Edit {
                before: 0,
                after: usize::from(joined),
                text: "\n\n".to_owned(),
            }
        }
    }
}

/// 返回在光标紧跟块标记（光标前只有标记）时按删除键要做的修改；否则返回 `None`，按普通删除
/// 处理。
///
/// `before` 是光标所在行从行首到光标的文字。任务框移除 `[ ]`；列表记号替换为等宽的空格，该项
/// 成为上一项中的另一段（第一项则成为段落）；引用移除最内层的 `>`；标题的 `#` 全部移除。预览中
/// 不显示标记原文，按普通删除会删掉记号后的空格，使 `- 甲` 变为 `-甲`，列表变成段落。
pub fn backspace(before: &str) -> Option<Edit> {
    let p = parse(before);
    let replace = |len: usize, text: String| Edit {
        before: len,
        after: 0,
        text,
    };
    // 标题：行首最多 3 个空格，1 到 6 个 `#`，再是空格。
    let body = p.rest.trim_start_matches(' ');
    let level = body.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&level)
        && p.rest.len() - body.len() <= 3
        && body[level..].starts_with(' ')
        && body[level..].trim().is_empty()
    {
        return Some(replace(p.rest.len(), String::new()));
    }
    if !p.rest.is_empty() {
        return None;
    }
    match &p.marker {
        Some(m) if m.task => {
            let bullet = p.quote.len() + p.indent.len() + m.bullet.len() + m.gap.len();
            Some(replace(before.len() - bullet, String::new()))
        }
        Some(m) => {
            let width = m.bullet.len() + m.gap.len();
            Some(replace(width, " ".repeat(width)))
        }
        None if !p.quote.is_empty() => {
            let outer = outer(p.quote);
            let text = if outer.is_empty() {
                String::new()
            } else {
                format!("{outer} ")
            };
            Some(replace(p.quote.len(), text))
        }
        None => None,
    }
}

/// 返回列表项 `line`（不含换行符）上按 Tab（`outdent` 为假）或 ⇧Tab 时要替换的范围和文字，
/// 用于将该项缩进为上一项的子项或退出一层。
///
/// `above` 是从上一行起向上的各行。返回的范围是缩进（字节偏移），替换文字是新的缩进；不是
/// 列表项时返回 `None`，按普通缩进处理。无法调整（第一项没有上一项、已在最外层）时替换为原文。
/// 只移动该行，子项和续行不随之移动。
pub fn shift<'a>(
    line: &str,
    above: impl Iterator<Item = &'a str>,
    outdent: bool,
) -> Option<(std::ops::Range<usize>, String)> {
    let p = parse(line);
    p.marker.as_ref()?;
    let range = p.quote.len()..p.quote.len() + p.indent.len();
    let mut items = above
        .take(LOOKBACK)
        .map(parse)
        .filter(|q| q.marker.is_some() && depth(q.quote) == depth(p.quote));
    let indent = if outdent {
        // 上一层：向上找到的第一个缩进更少的项。
        items
            .find(|q| q.indent.len() < p.indent.len())
            .map_or(String::new(), |q| q.indent.to_owned())
    } else {
        // 子项的缩进与上一项的内容对齐：`1. ` 之下缩进 3 个空格，`- ` 之下缩进 2 个空格。
        match items.find(|q| q.indent.len() <= p.indent.len()) {
            Some(q) if q.indent.len() == p.indent.len() => {
                let m = q.marker.as_ref()?;
                format!("{}{}", q.indent, " ".repeat(m.bullet.len() + m.gap.len()))
            }
            _ => p.indent.to_owned(),
        }
    };
    Some((range, indent))
}

/// 返回 ⇧ 回车要插入的文字：段内换行。
///
/// 位于列表项或引用中时，下一行继续同一项或同一段引用，行首带引用的 `>` 和与内容对齐的缩进。
pub fn line_break(before: &str) -> String {
    let p = parse(before);
    let indent = match &p.marker {
        Some(m) => format!("{}{}", p.indent, " ".repeat(m.width())),
        None => String::new(),
    };
    format!("\n{}{indent}", p.quote)
}

/// 返回在开围栏（```` ```rust ````、`~~~`、`$$`）末尾按回车时要插入到光标前后的文字。
///
/// 插入后光标位于两行围栏之间的空行。不是围栏、光标后还有文字、下方已有闭围栏时返回 `None`。
/// `below` 是下方的各行：代码块下方第一个围栏行只含反引号或波浪号时为闭围栏，带语言标识时是
/// 下一个代码块的开头，当前代码块未闭合。公式在下一个空行之前出现 `$$` 即为已闭合。
pub fn fence<'a>(
    before: &str,
    after: &str,
    below: impl Iterator<Item = &'a str>,
) -> Option<(String, String)> {
    if !after.trim().is_empty() {
        return None;
    }
    let p = parse(before);
    let body = p.rest.trim_start_matches(' ');
    if p.rest.len() - body.len() > 3 {
        return None;
    }
    let body = body.trim_end();
    let strip = |line: &'a str| line.trim_start_matches([' ', '\t', '>']);
    let fence = if body == "$$" {
        let closed = below
            .take_while(|l| !l.trim().is_empty())
            .any(|l| l.trim_end().ends_with("$$"));
        (!closed).then(|| "$$".to_owned())?
    } else {
        let c = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
        let n = body.chars().take_while(|&x| x == c).count();
        if n < 3 || (c == '`' && body[n..].contains('`')) {
            return None;
        }
        let next = below
            .map(strip)
            .find(|l| l.starts_with(&c.to_string().repeat(3)));
        let closed = next.is_some_and(|l| {
            let l = l.trim_end();
            l.chars().all(|x| x == c) && l.len() >= n
        });
        (!closed).then(|| c.to_string().repeat(n))?
    };
    let prefix = &line_break(before)[1..];
    Some((format!("\n{prefix}"), format!("\n{prefix}{fence}")))
}

#[cfg(test)]
mod tests;
