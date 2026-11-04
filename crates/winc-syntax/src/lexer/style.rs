//! 样式表（CSS、SCSS、Sass、Less）的规则：选择器与声明。
//!
//! 一行按 `{` 分成选择器和声明两部分：选择器着色类名、id、伪类和元素名，
//! 声明着色属性名和十六进制颜色。没有 `{` 的行按内容判断：以 `,` 结尾的是选择器，其余是声明。

use super::{Lexer, LineState, TokenKind, is_name, scan};

/// 是否是样式表标识符中的字符：字母、数字、`_` 或 `-`。
///
/// 连字符是属性名和类名的一部分（`background-color`、`nth-child`）。
fn is_css_name(c: char) -> bool {
    is_name(c) || c == '-'
}

impl Lexer<'_> {
    /// 按整行内容确定行首处于选择器还是声明中。
    ///
    /// `@` 开头的规则（`@media`、`@include`）两者都不是。上一行的声明值没有结束时
    /// （[`LineState::StyleValue`]），这一行从值的中间开始，以逗号结尾也不是选择器。
    pub(super) fn style_line_start(&mut self, state: LineState) {
        if state == LineState::StyleValue {
            self.selector = false;
            self.expect_property = false;
            self.in_value = true;
            return;
        }
        let chars = self.chars;
        let first = chars.iter().position(|c| !c.is_whitespace());
        let last = chars.iter().rposition(|c| !c.is_whitespace());
        let (Some(first), Some(last)) = (first, last) else {
            return;
        };
        if matches!(chars[first], '@' | '}') {
            self.selector = false;
            self.expect_property = false;
            return;
        }
        self.selector = chars.contains(&'{') || chars[last] == ',';
        self.expect_property = !self.selector;
    }

    /// 识别 `i` 处样式表特有的结构，返回其后的位置。
    ///
    /// 不是这类结构时返回 `None`，由通用规则处理。
    pub(super) fn style_token(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        let c = chars[i];
        match c {
            '{' => {
                self.selector = false;
                self.expect_property = true;
                self.in_value = false;
                return Some(i + 1);
            }
            '}' => {
                self.selector = chars[i + 1..].contains(&'{');
                self.expect_property = false;
                self.in_value = false;
                return Some(i + 1);
            }
            ';' => {
                self.expect_property = !self.selector;
                self.in_value = false;
                return Some(i + 1);
            }
            _ => {}
        }

        if self.selector {
            return self.selector_token(i);
        }

        // 十六进制颜色：`#fff`、`#1e90ff`、`#11223344`。
        if c == '#' {
            let end = scan(chars, i + 1, |c| c.is_ascii_hexdigit());
            let len = end - i - 1;
            if matches!(len, 3 | 4 | 6 | 8) && !chars.get(end).is_some_and(|&c| is_css_name(c)) {
                self.push(i, end, TokenKind::Number);
                return Some(end);
            }
            return None;
        }

        // 声明中 `{` 或 `;` 之后的属性名（`a { color: red; margin:0 }`）。
        let starts_name = c.is_alphabetic()
            || (c == '-'
                && chars
                    .get(i + 1)
                    .is_some_and(|&c| c.is_alphabetic() || c == '-'));
        if self.expect_property && starts_name {
            let end = scan(chars, i, is_css_name);
            self.expect_property = false;
            let colon = scan(chars, end, char::is_whitespace);
            if chars.get(colon) == Some(&':') {
                self.push(i, end, TokenKind::Property);
                self.in_value = true;
                return Some(end);
            }
            return None;
        }
        if !c.is_whitespace() {
            self.expect_property = false;
        }
        None
    }

    /// 识别选择器中 `i` 处的类名、id、伪类、父选择器或元素名，返回其后的位置；
    /// 都不是时返回 `None`。
    fn selector_token(&mut self, i: usize) -> Option<usize> {
        let chars = self.chars;
        match chars[i] {
            '.' | '#'
                if chars
                    .get(i + 1)
                    .is_some_and(|&c| c.is_alphabetic() || c == '_' || c == '-') =>
            {
                let end = scan(chars, i + 1, is_css_name);
                self.push(i, end, TokenKind::Attribute);
                Some(end)
            }
            ':' => {
                let name = scan(chars, i, |c| c == ':');
                let end = scan(chars, name, is_css_name);
                self.push(i, end, TokenKind::Builtin);
                Some(end.max(i + 1))
            }
            '&' => {
                self.push(i, i + 1, TokenKind::Keyword);
                Some(i + 1)
            }
            // 属性选择器 `[type="text"]` 中的名字不是元素名。
            '[' => Some(
                chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .map_or(i + 1, |k| i + k + 1),
            ),
            c if c.is_alphabetic() => {
                let end = scan(chars, i, is_css_name);
                let kind = if self.classify(i, end) == Some(TokenKind::Keyword) {
                    TokenKind::Keyword
                } else {
                    TokenKind::Tag
                };
                self.push(i, end, kind);
                Some(end)
            }
            _ => None,
        }
    }
}
