//! 裸网址自动变成链接（GFM 的扩展自动链接）：正文里直接写的 `https://…`、`http://…`、
//! `www.…` 不加尖括号也会变成链接。
//!
//! 规则取自 GFM：网址前面必须是开头、空白、`*_~(` 或中日韩文字；遇到空白、`<` 为止，末尾的
//! `?!.,:*_~`、`'"` 和没配对的 `)` 不计入。遇到中日韩文字和全角标点也为止：中文里常常紧挨着
//! 写「见 `https://x.dev`。」，句号不计入网址。

use super::{Inline, Run, Style, is_cjk};

/// 把 `inline` 里不在链接和行内代码中的裸网址标成链接。
pub(super) fn autolink(inline: &mut Inline) {
    if !inline.text.contains("http") && !inline.text.contains("www.") {
        return;
    }
    let mut runs = Vec::with_capacity(inline.runs.len());
    let mut start = 0;
    for run in std::mem::take(&mut inline.runs) {
        let style = run.style;
        if style.link != 0 || style.has(Style::CODE) || style.has(Style::MATH) {
            runs.push(run);
            start = run.end;
            continue;
        }
        let mut at = start;
        while let Some((from, to)) = find(&inline.text, at, run.end) {
            let url = &inline.text[from..to];
            let url = if url.starts_with("www.") {
                format!("http://{url}")
            } else {
                url.to_owned()
            };
            inline.links.push(url);
            let link = u16::try_from(inline.links.len()).unwrap_or(u16::MAX);
            if from > at {
                runs.push(Run { end: from, style });
            }
            runs.push(Run {
                end: to,
                style: Style { link, ..style },
            });
            at = to;
        }
        if run.end > at {
            runs.push(Run {
                end: run.end,
                style,
            });
        }
        start = run.end;
    }
    inline.runs = runs;
}

/// 网址末尾不计入的标点；`)` 另按是否配对处理。
const TRAILING: &str = "?!.,:*_~'\"";

/// 在 `text[from..to]` 里找第一个网址，返回它的字节范围。
///
/// 每个候选都只花常数时间（均摊）：记下各前缀下一次出现的位置，`at` 越过时才重新查找；同一段
/// 不含空白的文字（`Token`）里的候选共用同一次扫描的结果。若每个候选都从头扫描，
/// `www.www.www.…` 这样的文字是平方级。
fn find(text: &str, from: usize, to: usize) -> Option<(usize, usize)> {
    const PREFIXES: [&str; 3] = ["https://", "http://", "www."];
    // 各前缀不早于 `at` 的下一次出现；没有时为 `to`。
    let mut next: [Option<usize>; 3] = [None; 3];
    let mut token: Option<Token> = None;
    let mut at = from;
    while at < to {
        let mut best: Option<(usize, usize)> = None;
        for (slot, prefix) in next.iter_mut().zip(PREFIXES) {
            let pos = match *slot {
                Some(pos) if pos >= at => pos,
                _ => {
                    let pos = text[at..to].find(prefix).map_or(to, |i| at + i);
                    *slot = Some(pos);
                    pos
                }
            };
            if pos < to && best.is_none_or(|(b, _)| pos < b) {
                best = Some((pos, prefix.len()));
            }
        }
        let (begin, prefix) = best?;
        let before = text[..begin].chars().next_back();
        if before.is_none_or(|c| c.is_whitespace() || "*_~(".contains(c) || is_cjk(c)) {
            let body = begin + prefix;
            let token = match &mut token {
                Some(t) if body <= t.end => t,
                slot => slot.insert(Token::new(text, body, to)),
            };
            if let Some(end) = token.url_end(text, body)
                && token.host_ok(text, body, end, prefix == 4)
            {
                return Some((begin, end));
            }
        }
        at = begin + prefix;
    }
    None
}

/// 一段不含空白（以及 `<`、中日韩文字）的文字：其中各候选网址的终点、主机名都在这里查找，
/// 候选从前往后移动，记下的位置只往后移。
struct Token {
    /// 这一段的终点（字节偏移）。
    end: usize,
    /// 末尾那一串可能被去掉的字符（`TRAILING` 和 `)`）的起点（字节偏移）。
    run: usize,
    /// 那一串里各个 `)` 的位置，从前往后。
    closes_in_run: Vec<usize>,
    /// `(` 的个数与 `)` 的个数，统计范围是 `counted` 到 `run`；`counted` 为 `None` 时尚未统计。
    counted: Option<usize>,
    opens: usize,
    closes: usize,
    /// 不早于上次查找处的下一个主机名分隔符（`/?#:`）、不能出现在主机名里的字符、点。
    delimiter: Option<usize>,
    bad: Option<usize>,
    dot: Option<usize>,
}

impl Token {
    /// 从 `from` 起找到这一段的终点（不超过 `to`），记下末尾的标点串。
    fn new(text: &str, from: usize, to: usize) -> Token {
        let body = &text[from..to];
        let end = from
            + body
                .find(|c: char| {
                    c.is_whitespace() || c == '<' || is_cjk(c) || c == super::IMAGE_MARK
                })
                .unwrap_or(body.len());
        let mut run = end;
        for (i, c) in text[from..end].char_indices().rev() {
            if c != ')' && !TRAILING.contains(c) {
                break;
            }
            run = from + i;
        }
        let closes_in_run = text[run..end]
            .match_indices(')')
            .map(|(i, _)| run + i)
            .collect();
        Token {
            end,
            run,
            closes_in_run,
            counted: None,
            opens: 0,
            closes: 0,
            delimiter: None,
            bad: None,
            dot: None,
        }
    }

    /// 网址正文从 `body` 起时，返回去掉末尾标点之后的终点；什么也不剩时返回 `None`。
    ///
    /// 末尾的标点总是去掉；`)` 从后往前，比 `(` 多时去掉。末尾那一串里没有 `(`：设那一串之前
    /// `(` 比 `)` 多 k 个，那一串里留下前 k 个 `)`，终点直接算出，无需逐个去掉。
    fn url_end(&mut self, text: &str, body: usize) -> Option<usize> {
        if body >= self.run {
            return None;
        }
        match self.counted {
            Some(counted) if counted <= body => {
                for c in text[counted..body].chars() {
                    self.opens -= usize::from(c == '(');
                    self.closes -= usize::from(c == ')');
                }
            }
            _ => {
                let span = &text[body..self.run];
                self.opens = span.matches('(').count();
                self.closes = span.matches(')').count();
            }
        }
        self.counted = Some(body);
        let kept = self.opens.saturating_sub(self.closes);
        Some(match kept.min(self.closes_in_run.len()) {
            0 => self.run,
            n => self.closes_in_run[n - 1] + 1,
        })
    }

    /// `[body, end)` 的主机名（到 `/?#:` 为止）是否有效：不空、只由字母数字和 `-_.` 组成；
    /// `www` 为真时（`www.` 开头）还要带点。
    fn host_ok(&mut self, text: &str, body: usize, end: usize, www: bool) -> bool {
        let limit = self.end;
        let seek = |slot: &mut Option<usize>, hit: fn(char) -> bool| match *slot {
            Some(pos) if pos >= body => pos,
            _ => {
                let pos = text[body..limit]
                    .char_indices()
                    .find(|&(_, c)| hit(c))
                    .map_or(limit, |(i, _)| body + i);
                *slot = Some(pos);
                pos
            }
        };
        let host_end = seek(&mut self.delimiter, |c| "/?#:".contains(c)).min(end);
        let bad = seek(&mut self.bad, |c| {
            !c.is_alphanumeric() && !"-_.".contains(c)
        });
        // `www.` 后面必须有一段带点的域名，`https://` 后面至少要有主机名。
        host_end > body && bad >= host_end && (!www || seek(&mut self.dot, |c| c == '.') < host_end)
    }
}
