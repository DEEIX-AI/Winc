//! GFM 的 emoji 短码：正文里写 `:tada:` 显示成 🎉。
//!
//! 名字表取自 gemoji 项目（MIT 许可）的 `db/emoji.json`，每行依次是名字、制表符和 emoji，
//! 按名字排序后用二分查找。只收有 Unicode 字符的，`:octocat:` 这类图标没有。

use std::ops::Range;

use super::{Inline, Run, Style};

/// 名字表，见模块说明。
const TABLE: &str = include_str!("emoji.txt");

/// 返回短码 `name`（不带冒号）对应的 emoji；没有这个短码时返回 `None`。
fn lookup(name: &str) -> Option<&'static str> {
    // 每行的开头位置；首次使用时生成并缓存。
    static STARTS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    let starts = STARTS.get_or_init(|| {
        std::iter::once(0)
            .chain(TABLE.match_indices('\n').map(|(i, _)| i as u32 + 1))
            .filter(|&i| (i as usize) < TABLE.len())
            .collect()
    });
    let line = |i: usize| {
        let from = starts[i] as usize;
        let line = TABLE[from..].split('\n').next().unwrap_or_default();
        line.split_once('\t').unwrap_or((line, ""))
    };
    let i = starts
        .binary_search_by(|&s| {
            let from = s as usize;
            let rest = &TABLE[from..];
            rest[..rest.find('\t').unwrap_or(rest.len())].cmp(name)
        })
        .ok()?;
    Some(line(i).1)
}

/// 短码名字里允许出现的字符。
fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// 把 `inline` 里不在行内代码中的 `:name:` 换成 emoji，样式段随之移动。
pub(super) fn shortcodes(inline: &mut Inline) {
    if !inline.text.contains(':') {
        return;
    }
    let old = std::mem::take(&mut inline.text);
    let mut runs = Vec::with_capacity(inline.runs.len());
    // 换掉的短码：原来的位置范围和换成的 emoji 的字节数，原文对照按它们移动。
    let mut edits = Vec::new();
    let mut start = 0;
    for run in std::mem::take(&mut inline.runs) {
        let piece = &old[start..run.end];
        if run.style.has(Style::CODE) || run.style.has(Style::MATH) {
            inline.text.push_str(piece);
        } else {
            replace(piece, start, &mut inline.text, &mut edits);
        }
        runs.push(Run {
            end: inline.text.len(),
            style: run.style,
        });
        start = run.end;
    }
    inline.runs = runs;
    // 从后往前挪：前面的范围不受后面替换的影响。
    for (range, len) in edits.into_iter().rev() {
        inline.map.replace(range, len);
    }
}

/// 把 `text`（原来从第 `base` 个字节起）里的短码替换后接到 `out` 后面，并把换掉的范围（按原来
/// 的位置）和 emoji 的字节数记到 `edits`。
fn replace(text: &str, base: usize, out: &mut String, edits: &mut Vec<(Range<usize>, usize)>) {
    let mut rest = text;
    while let Some(open) = rest.find(':') {
        let after = &rest[open + 1..];
        let len = after.find(|c: char| !is_name(c)).unwrap_or(after.len());
        let found = (len > 0 && after[len..].starts_with(':'))
            .then(|| lookup(&after[..len]))
            .flatten();
        match found {
            Some(emoji) => {
                let at = base + text.len() - rest.len() + open;
                edits.push((at..at + len + 2, emoji.len()));
                out.push_str(&rest[..open]);
                out.push_str(emoji);
                rest = &after[len + 1..];
            }
            // 不是短码：冒号原样留下，从下一个字继续找（`a:b:tada:` 里的 `:b:` 不是短码，
            // `:tada:` 仍然识别）。
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
}
