//! `<style>` 中的 CSS：选择器（标签、`*`、`.类`、`#id`、`[属性]`、`[属性=值]`、`[属性~=值]`、
//! `:first-child`、`:last-child`、`:nth-child(an+b)`，后代组合符 ` ` 和子元素组合符 `>`）、
//! 优先级和声明。
//!
//! `@media` 等以 `@` 开头的规则块整体跳过。

use std::borrow::Cow;
use std::collections::HashMap;

use roxmltree::Node;

/// 一个样式表：文件中所有 `<style>` 里的规则，以及按最右边一段简单选择器建立的索引。
///
/// 每条规则按最右边一段放进一个桶：有 id 的按 id，否则有类的按第一个类，否则有标签的按标签，
/// 其余（`*`、只有属性选择器或伪类）放进 `universal`。元素只可能匹配自己的 id、各个类、标签
/// 对应的桶和 `universal` 中的规则，其余规则不必逐条尝试：绘图软件导出的文件常有数百条
/// `.st12 { … }` 这样的规则和数万个元素。
#[derive(Debug, Default)]
pub(super) struct Sheet {
    /// 按在样式表中的先后顺序排列的规则；优先级相同时，下标大的优先。
    rules: Vec<Rule>,
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

/// 一条规则：一个选择器及其声明。
#[derive(Debug)]
struct Rule {
    /// 从左到右排列的（组合符，简单选择器）；第一项的组合符是 [`Combinator::None`]。
    selector: Vec<(Combinator, Compound)>,
    /// 优先级（id 数，类、属性和伪类数，标签数），按元组字典序比较。
    specificity: (usize, usize, usize),
    /// （属性名，值）列表，属性名为小写。
    declarations: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Combinator {
    /// 最左边的简单选择器，前面没有组合符。
    None,
    /// 后代组合符（空格）。
    Descendant,
    /// 子元素组合符（`>`）。
    Child,
}

#[derive(Debug, Default)]
struct Compound {
    /// 标签名；`*` 或未写时为 `None`。
    tag: Option<String>,
    /// `#id` 选择器的 id。
    id: Option<String>,
    /// 各个 `.类` 选择器的类名。
    classes: Vec<String>,
    /// 属性选择器，元素为（属性名，值，是否按空白分隔的单词匹配）：`[name]` 的值是 `None`，
    /// `[name=value]` 要求相等，`[name~=value]` 的第三项为 `true`。
    attributes: Vec<(String, Option<String>, bool)>,
    /// 是否要求是第一个子元素（`:first-child`）。
    first_child: bool,
    /// 是否要求是最后一个子元素（`:last-child`）。
    last_child: bool,
    /// `:nth-child(an+b)` 中的 (a, b)。
    nth_child: Option<(i32, i32)>,
}

impl Sheet {
    /// 是否在样式表 `css` 中遇到了不支持的选择器。
    ///
    /// 调用时把 `css` 中的规则追加到样式表；含不支持选择器的规则被跳过。以 `@` 开头的规则块
    /// （`@media`、`@supports`、`@font-face` 等）整体跳过，不计入返回值。
    pub(super) fn add(&mut self, css: &str) -> bool {
        let css = strip_comments(css);
        let mut unknown = false;
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let selectors = rest[..open].trim();
            // 先按大括号配对取出整个块，使带嵌套块的 `@` 规则（`@media`、`@supports`）能整体跳过；
            // `@font-face` 等只有一层的规则也一并跳过。
            let Some(close) = matching_brace(&rest[open..]) else {
                break;
            };
            let body = &rest[open + 1..open + close];
            rest = &rest[open + close + 1..];
            if selectors.starts_with('@') || selectors.is_empty() {
                continue;
            }
            let declarations: Vec<(String, String)> = declarations(body)
                .into_iter()
                .map(|(k, v)| (k.into_owned(), v.to_owned()))
                .collect();
            for selector in selectors.split(',') {
                match parse_selector(selector.trim()) {
                    Some(selector) => self.push(Rule {
                        specificity: specificity(&selector),
                        selector,
                        declarations: declarations.clone(),
                    }),
                    None => unknown = true,
                }
            }
        }
        unknown
    }

    /// 追加一条规则，并按它最右边的一段放进对应的桶。
    fn push(&mut self, rule: Rule) {
        let index = self.rules.len();
        let bucket = match rule.selector.last().map(|(_, c)| c) {
            Some(Compound { id: Some(id), .. }) => self.by_id.entry(id.clone()).or_default(),
            Some(Compound { classes, .. }) if !classes.is_empty() => {
                self.by_class.entry(classes[0].clone()).or_default()
            }
            Some(Compound { tag: Some(tag), .. }) => self.by_tag.entry(tag.clone()).or_default(),
            _ => &mut self.universal,
        };
        bucket.push(index);
        self.rules.push(rule);
    }

    /// 返回匹配 `node` 的规则中的声明，按优先级从低到高排列；优先级相同时按规则的先后顺序。
    pub(super) fn matching<'s>(
        &'s self,
        node: Node,
    ) -> impl Iterator<Item = (&'s str, &'s str)> + use<'s> {
        let mut candidates: Vec<usize> = Vec::new();
        if !self.rules.is_empty() {
            candidates.extend(&self.universal);
            if let Some(bucket) = node.attribute("id").and_then(|id| self.by_id.get(id)) {
                candidates.extend(bucket);
            }
            for class in node.attribute("class").unwrap_or("").split_whitespace() {
                if let Some(bucket) = self.by_class.get(class) {
                    candidates.extend(bucket);
                }
            }
            if let Some(bucket) = self.by_tag.get(node.tag_name().name()) {
                candidates.extend(bucket);
            }
            // 元素的 `class` 里同一个类可以写两次，同一个桶就会取到两次。
            candidates.sort_unstable();
            candidates.dedup();
            candidates.retain(|&i| matches(&self.rules[i].selector, node));
            candidates.sort_by_key(|&i| (self.rules[i].specificity, i));
        }
        candidates.into_iter().flat_map(move |i| {
            self.rules[i]
                .declarations
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
        })
    }
}

/// 返回选择器的优先级（id 数，类、属性和伪类数，标签数）。
fn specificity(selector: &[(Combinator, Compound)]) -> (usize, usize, usize) {
    selector.iter().fold((0, 0, 0), |(a, b, c), (_, s)| {
        (
            a + usize::from(s.id.is_some()),
            b + s.classes.len()
                + s.attributes.len()
                + usize::from(s.first_child)
                + usize::from(s.last_child)
                + usize::from(s.nth_child.is_some()),
            c + usize::from(s.tag.is_some()),
        )
    })
}

/// 返回 `text`（以 `{` 开头）中与首个 `{` 配对的 `}` 的字节偏移，嵌套的块一并计入；
/// 没有配对时返回 `None`。
fn matching_brace(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// 解析逗号分隔列表中的一项选择器，返回（组合符，简单选择器）序列。
///
/// 含相邻兄弟 `+`、通用兄弟 `~` 组合符或其他不支持的语法时返回 `None`。
fn parse_selector(text: &str) -> Option<Vec<(Combinator, Compound)>> {
    if text.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut combinator = Combinator::None;
    let mut rest = text;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(r) = rest.strip_prefix('>') {
            if out.is_empty() {
                return None;
            }
            combinator = Combinator::Child;
            rest = r;
            continue;
        }
        if rest.starts_with(['+', '~']) {
            return None;
        }
        let end = rest
            .find([' ', '>', '+', '~', '\t', '\n'])
            .unwrap_or(rest.len());
        out.push((combinator, compound(&rest[..end])?));
        rest = &rest[end..];
        combinator = Combinator::Descendant;
    }
    (!out.is_empty()).then_some(out)
}

/// 解析不含空白和组合符的一段简单选择器，如 `path.x[a=b]:first-child`。
///
/// 含不支持的语法（`^=`、`$=`、`*=`、`|=` 属性运算符，或 `:first-child`、`:last-child`、
/// `:nth-child` 之外的伪类）时返回 `None`。
fn compound(text: &str) -> Option<Compound> {
    let mut c = Compound::default();
    let name_end = text.find(['.', '#', '[', ':']).unwrap_or(text.len());
    match &text[..name_end] {
        "" | "*" => {}
        tag => c.tag = Some(tag.to_owned()),
    }
    let mut rest = &text[name_end..];
    while let Some(kind) = rest.chars().next() {
        // 按字符而不是按字节前进：在多字节字符中间切开字符串会 panic。
        rest = &rest[kind.len_utf8()..];
        match kind {
            '.' | '#' => {
                let end = rest.find(['.', '#', '[', ':']).unwrap_or(rest.len());
                let name = rest[..end].to_owned();
                if name.is_empty() {
                    return None;
                }
                if kind == '#' {
                    c.id = Some(name);
                } else {
                    c.classes.push(name);
                }
                rest = &rest[end..];
            }
            '[' => {
                let end = rest.find(']')?;
                let inside = &rest[..end];
                rest = &rest[end + 1..];
                let unquote = |v: &str| v.trim().trim_matches(['"', '\'']).to_owned();
                if let Some((name, value)) = inside.split_once("~=") {
                    c.attributes
                        .push((name.trim().to_owned(), Some(unquote(value)), true));
                } else if let Some((name, value)) = inside.split_once('=') {
                    if name.ends_with(['^', '$', '*', '|']) {
                        return None;
                    }
                    c.attributes
                        .push((name.trim().to_owned(), Some(unquote(value)), false));
                } else {
                    c.attributes.push((inside.trim().to_owned(), None, false));
                }
            }
            ':' => {
                if let Some(args) = rest.strip_prefix("nth-child(") {
                    let close = args.find(')')?;
                    c.nth_child = Some(nth(&args[..close])?);
                    rest = &args[close + 1..];
                    continue;
                }
                let end = rest.find(['.', '#', '[', ':']).unwrap_or(rest.len());
                match &rest[..end] {
                    "first-child" => c.first_child = true,
                    "last-child" => c.last_child = true,
                    _ => return None,
                }
                rest = &rest[end..];
            }
            _ => return None,
        }
    }
    Some(c)
}

/// 把 `:nth-child` 的参数（`odd`、`even`、`3`、`2n+1`、`-n+3` 等）解析为 (a, b)，表示匹配
/// 第 a·k + b 个元素（k = 0, 1, 2 …，序号从 1 起）；无法解析时返回 `None`。
fn nth(text: &str) -> Option<(i32, i32)> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    Some(match t.as_str() {
        "odd" => (2, 1),
        "even" => (2, 0),
        _ => match t.split_once('n') {
            None => (0, t.parse().ok()?),
            Some((a, b)) => {
                let a = match a {
                    "" | "+" => 1,
                    "-" => -1,
                    a => a.parse().ok()?,
                };
                let b = if b.is_empty() { 0 } else { b.parse().ok()? };
                (a, b)
            }
        },
    })
}

/// `node` 是否满足简单选择器 `c`；`node` 不是元素时为 `false`。
fn matches_compound(c: &Compound, node: Node) -> bool {
    if !node.is_element() {
        return false;
    }
    if c.tag
        .as_deref()
        .is_some_and(|t| t != node.tag_name().name())
    {
        return false;
    }
    if c.id
        .as_deref()
        .is_some_and(|id| node.attribute("id") != Some(id))
    {
        return false;
    }
    if !c.classes.is_empty() {
        let classes = node.attribute("class").unwrap_or("");
        if !c
            .classes
            .iter()
            .all(|want| classes.split_whitespace().any(|have| have == want))
        {
            return false;
        }
    }
    for (name, value, word) in &c.attributes {
        let Some(have) = node.attribute(name.as_str()) else {
            return false;
        };
        match value {
            Some(v) if *word => {
                if !have.split_whitespace().any(|w| w == v) {
                    return false;
                }
            }
            Some(v) if have != v => return false,
            _ => {}
        }
    }
    if c.first_child && node.prev_sibling_element().is_some() {
        return false;
    }
    if c.last_child && node.next_sibling_element().is_some() {
        return false;
    }
    if let Some((a, b)) = c.nth_child {
        // `prev_siblings` 包含节点自身，因此 `position` 是节点在兄弟元素中的序号（从 1 起）。
        // 用 `i64` 计算：`b` 可以小到 `i32::MIN`，`position - b` 用 `i32` 会溢出。
        let position = node.prev_siblings().filter(Node::is_element).count() as i64;
        let (a, b) = (i64::from(a), i64::from(b));
        let n = position - b;
        let ok = if a == 0 {
            n == 0
        } else {
            n % a == 0 && n / a >= 0
        };
        if !ok {
            return false;
        }
    }
    true
}

/// 选择器的匹配结果，并指出不匹配时换更上层的祖先重试是否有意义。
#[derive(Clone, Copy, PartialEq)]
enum Outcome {
    Matched,
    /// 该元素不匹配，换更上层的祖先可能匹配。
    NotHere,
    /// 无论换哪个祖先都不会匹配：左边的某一段在所有祖先中都找不到。
    Never,
}

/// 选择器 `selector` 是否匹配 `node`。
fn matches(selector: &[(Combinator, Compound)], node: Node) -> bool {
    matches_from(selector, node) == Outcome::Matched
}

/// 从右往左匹配选择器：最右边的简单选择器匹配 `node`，再向上匹配左边的各段。
///
/// 后代选择器要尝试每一个祖先，层层嵌套地尝试，耗时随选择器中后代段的数量指数增长
/// （`g g g g g x` 对着数百层的 `<g>`）。左边的某一段在所有祖先中都找不到（[`Outcome::Never`]）时，
/// 外层不必再换祖先：换到更上层的祖先，它的祖先集合只会更小。
fn matches_from(selector: &[(Combinator, Compound)], node: Node) -> Outcome {
    let Some(((combinator, last), rest)) = selector.split_last() else {
        return Outcome::Matched;
    };
    if !matches_compound(last, node) {
        return Outcome::NotHere;
    }
    match combinator {
        Combinator::None => Outcome::Matched,
        Combinator::Child => match node.parent_element() {
            Some(parent) => matches_from(rest, parent),
            None => Outcome::Never,
        },
        Combinator::Descendant => {
            for ancestor in node.ancestors().skip(1).filter(Node::is_element) {
                match matches_from(rest, ancestor) {
                    Outcome::NotHere => {}
                    found => return found,
                }
            }
            Outcome::Never
        }
    }
}

/// 删除 CSS 注释；注释没有闭合时，丢弃其后的全部内容。
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// 把 `a: b; c: d` 拆成（属性名，值）列表，属性名转为小写。
///
/// 名和值直接引用 `body`，只有含大写字母的属性名才另存一个小写的副本。
///
/// 值末尾的 `!important` 被去掉，不区分声明的重要性。
pub(super) fn declarations(body: &str) -> Vec<(Cow<'_, str>, &str)> {
    body.split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| {
            let k = k.trim();
            let k = if k.bytes().any(|b| b.is_ascii_uppercase()) {
                Cow::Owned(k.to_ascii_lowercase())
            } else {
                Cow::Borrowed(k)
            };
            (k, v.trim().trim_end_matches("!important").trim())
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

#[cfg(test)]
mod tests;
