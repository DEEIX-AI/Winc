//! 语言服务器推送的诊断：按文档存放，范围换算成编辑器统一的字符偏移。
//!
//! 服务器按（行号，UTF-16 列）给出范围，编辑器全库用字符偏移；换算在本模块用文档当前的
//! 文本完成（服务器推送可能略落后于文档，换算时把越界的位置夹到文本末尾）。诊断整表替换：
//! 每来一次推送就换掉该文档的全部诊断，与推送本身的一致性语义相同（服务器只发全量）。

use std::collections::HashMap;

use winc_core::Lines;
use winc_editor::{Document, DocumentId};
use winc_lsp::message::{Diagnostic, Severity};
use winc_lsp::position;


/// 一条诊断：字符范围、严重级别和消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    /// 范围的起点（字符偏移）。
    pub start: usize,
    /// 范围的终点（字符偏移，不含）。
    pub end: usize,
    /// 严重级别。
    pub severity: Severity,
    /// 消息文本。
    pub message: String,
}

/// 一行的诊断范围：行内起点、终点（字符，半开区间）与严重级别。
pub(crate) type LineRange = (usize, usize, Severity);

/// 绘制用的一行诊断：行内起止字符（从 0 起，半开区间）与颜色。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Underline {
    /// 起点（字符）。
    pub start: usize,
    /// 终点（字符，不含）。
    pub end: usize,
    /// 错误或警告的颜色（取自主题）。
    pub color: winc_render::Color,
}

/// 各文档的诊断，整表替换。
#[derive(Default)]
pub(crate) struct Diagnostics {
    /// 按文档存放；没有诊断的文档不在表中。键是文档标识的数值（`DocumentId` 不实现 `Ord`）。
    by_doc: HashMap<u64, Vec<Problem>>,
}

impl Diagnostics {
    /// 用 `problems` 替换文档 `doc` 的全部诊断；空表移除该文档的记录。
    pub(crate) fn replace(&mut self, doc: DocumentId, mut problems: Vec<Problem>) {
        if problems.is_empty() {
            self.by_doc.remove(&doc.0);
            return;
        }
        problems.sort_by_key(|p| (p.start, p.end));
        self.by_doc.insert(doc.0, problems);
    }

    /// 返回文档 `doc` 的诊断，按范围起点排序；没有时为 `None`。
    pub(crate) fn get(&self, doc: DocumentId) -> Option<&[Problem]> {
        self.by_doc.get(&doc.0).map(Vec::as_slice)
    }

    /// 只保留 `keep` 认可的文档的诊断（关闭的文档不再显示波浪线）。
    pub(crate) fn retain(&mut self, keep: impl Fn(DocumentId) -> bool) {
        self.by_doc.retain(|id, _| keep(DocumentId(*id)));
    }

    /// 返回文档 `doc` 的诊断按行分组的结果：行号、逐行的字符范围与严重级别。
    ///
    /// 跨行的诊断在每一行各占一段；只取可见范围内的行由绘制方负责（这里只按行分好）。
    pub(crate) fn by_line(&self, doc: &Document) -> Vec<(usize, Vec<LineRange>)> {
        let Some(problems) = self.get(doc.id) else {
            return Vec::new();
        };
        let text = doc.text();
        let mut lines: std::collections::BTreeMap<usize, Vec<LineRange>> =
            std::collections::BTreeMap::new();
        for problem in problems {
            let start = problem.start.min(text.end_pos());
            let end = problem.end.max(start).min(text.end_pos());
            let first = text.line_of(start);
            let last = if end > start { text.line_of(end - 1) } else { first };
            for line in first..=last {
                let line_start = text.line_start(line);
                let line_end = line_start + text.line_len(line);
                let a = start.max(line_start) - line_start;
                let b = end.min(line_end) - line_start;
                // 零宽的诊断（例如行尾缺分号）至少占一个字符的位置；整行为空时从 0 起。
                let (a, b) = if b > a {
                    (a, b)
                } else {
                    (a.saturating_sub(1), a)
                };
                lines.entry(line).or_default().push((a, b, problem.severity));
            }
        }
        lines.into_iter().collect()
    }

    /// 返回范围包含 `caret`（字符偏移）的诊断；范围两端都算包含。
    pub(crate) fn at(&self, doc: DocumentId, caret: usize) -> Vec<&Problem> {
        self.get(doc)
            .unwrap_or_default()
            .iter()
            .filter(|p| p.start <= caret && caret <= p.end)
            .collect()
    }
}

/// 把服务器推送的诊断换算成字符范围；文档当前的文本用于换算，越界的位置夹到文本末尾。
pub(crate) fn convert(doc: &Document, diagnostics: &[Diagnostic]) -> Vec<Problem> {
    let text = doc.text();
    diagnostics
        .iter()
        .map(|d| Problem {
            start: char_offset(text, d.range.start),
            end: char_offset(text, d.range.end),
            severity: d.severity,
            message: d.message.clone(),
        })
        .filter(|p| p.start <= p.end)
        .collect()
}

/// 把 LSP 位置换算成字符偏移：行号越界时取最后一行，列越界时取行尾。
fn char_offset(text: &impl Lines, at: winc_lsp::message::Position) -> usize {
    let line = (at.line as usize).min(text.line_count().saturating_sub(1));
    let start = text.line_start(line);
    let content = text.line_content(line);
    let line_text = content.slice().to_string();
    start + position::char_in_line(at, &line_text).min(text.line_len(line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use winc_lsp::message::{Position, Range};

    fn doc(text: &str) -> Document {
        Document::from_text(DocumentId(1), text)
    }

    fn range(start: (u32, u32), end: (u32, u32)) -> Range {
        Range {
            start: Position { line: start.0, character: start.1 },
            end: Position { line: end.0, character: end.1 },
        }
    }

    fn problem(start: usize, end: usize, severity: Severity, message: &str) -> Problem {
        Problem { start, end, severity, message: message.into() }
    }

    #[test]
    fn positions_are_converted_from_utf16_columns_to_char_offsets() {
        let doc = doc("abc\n中文🙂x\n");
        let problems = convert(
            &doc,
            &[Diagnostic {
                range: range((1, 0), (1, 4)),
                severity: Severity::Error,
                message: "错误".into(),
            }],
        );
        // 第二行从字符 4 起；「中文🙂」在 UTF-16 中占 4 个 code unit、3 个字符。
        assert_eq!(problems, [problem(4, 7, Severity::Error, "错误")]);
    }

    #[test]
    fn multi_line_ranges_and_out_of_bounds_positions_are_clamped() {
        let doc = doc("ab\ncd\n");
        let problems = convert(
            &doc,
            &[
                Diagnostic {
                    range: range((0, 1), (1, 2)),
                    severity: Severity::Warning,
                    message: "跨行".into(),
                },
                // 行号、列都超出文本末尾的推送（文档比服务器新）。
                Diagnostic {
                    range: range((9, 9), (9, 9)),
                    severity: Severity::Error,
                    message: "越界".into(),
                },
            ],
        );
        assert_eq!(problems[0], problem(1, 5, Severity::Warning, "跨行"));
        let end = doc.text().end_pos();
        assert_eq!(problems[1], problem(end, end, Severity::Error, "越界"));
    }

    #[test]
    fn a_push_replaces_the_whole_table_for_the_document() {
        let mut all = Diagnostics::default();
        all.replace(DocumentId(1), vec![problem(0, 1, Severity::Error, "旧")]);
        all.replace(DocumentId(1), vec![problem(2, 3, Severity::Warning, "新")]);
        assert_eq!(all.get(DocumentId(1)).unwrap().len(), 1);
        assert_eq!(all.get(DocumentId(1)).unwrap()[0].message, "新");
        // 空推送清空该文档。
        all.replace(DocumentId(1), Vec::new());
        assert!(all.get(DocumentId(1)).is_none());
    }

    #[test]
    fn diagnostics_at_the_caret_include_both_ends() {
        let mut all = Diagnostics::default();
        all.replace(
            DocumentId(1),
            vec![
                problem(0, 3, Severity::Warning, "警告"),
                problem(2, 4, Severity::Error, "错误"),
                problem(5, 6, Severity::Error, "外面"),
            ],
        );
        let hit = all.at(DocumentId(1), 3);
        assert_eq!(hit.len(), 2);
        assert!(all.at(DocumentId(2), 3).is_empty());
    }

    /// 跨行的诊断在每一行各占一段；零宽诊断至少占一个字符。
    #[test]
    fn a_multi_line_diagnostic_covers_each_line() {
        let doc = doc("abc
defg
hij
");
        let mut all = Diagnostics::default();
        all.replace(
            DocumentId(1),
            convert(
                &doc,
                &[Diagnostic {
                    range: range((0, 1), (1, 2)),
                    severity: Severity::Error,
                    message: "跨行".into(),
                }],
            ),
        );
        let lines = all.by_line(&doc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, 0);
        assert_eq!(lines[0].1, [(1, 3, Severity::Error)]);
        assert_eq!(lines[1].0, 1);
        assert_eq!(lines[1].1, [(0, 2, Severity::Error)]);

        // 行尾的零宽诊断：画在最后一个字符上。
        let mut all = Diagnostics::default();
        let at = doc.text().line_end(1);
        all.replace(DocumentId(1), vec![problem(at, at, Severity::Warning, "缺分号")]);
        let lines = all.by_line(&doc);
        assert_eq!(lines, [(1, vec![(3, 4, Severity::Warning)])]);
    }

    #[test]
    fn problems_are_sorted_by_start() {
        let mut all = Diagnostics::default();
        all.replace(
            DocumentId(1),
            vec![
                problem(6, 7, Severity::Error, "后"),
                problem(0, 1, Severity::Error, "前"),
            ],
        );
        let starts: Vec<usize> = all.get(DocumentId(1)).unwrap().iter().map(|p| p.start).collect();
        assert_eq!(starts, [0, 6]);
    }
}
