//! 语义补全的会话状态：何时触发、防抖、候选的过滤与取舍。
//!
//! 只处理「什么时候请求、请求到了给谁看、已输入的字符怎样筛选」；把候选写进文档由接受一方
//! （`app/editor.rs` 的 `accept_completion`）完成，列表的绘制在 `view/completion.rs`。真正的
//! 请求发送（JSON-RPC 的 id 与连接）在 `app/lsp.rs`，本模块只产出 [`Context`] 并记住发出去的 id。
//!
//! 触发检测与防抖都按「每帧看一眼」的方式设计，与 Markdown 的链接补全（`suggest`）同型：
//! 文档没变、列表也没开时直接返回，开销只是一次整数比较。

use std::time::{Duration, Instant};

use winc_core::Lines;
use winc_editor::{Document, DocumentId};
use winc_lsp::message::CompletionItem;

/// 从停止输入到发出请求的等待（毫秒，取自调参）：连续输入时只请求最后一次。
pub(crate) const DELAY: Duration = Duration::from_millis(120);
/// 一次最多保留的候选数：再多的候选也没法在一屏里找到，过滤与绘制都随它增长。
pub(crate) const LIMIT: usize = 200;

/// 一次补全的上下文：在哪个文档的哪个位置、已输入的词是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Context {
    /// 文档。
    pub doc: DocumentId,
    /// 文档版本号：响应回来时据此判断是否过期。
    pub revision: u64,
    /// 光标位置（字符偏移），请求的位置。
    pub caret: usize,
    /// 已输入的词的开头（字符偏移），接受时没有 `textEdit` 就替换这一段。
    pub start: usize,
    /// 已输入的词。
    pub prefix: String,
    /// 触发字符（`.` 之类）；按输入标识符触发时为 `None`。
    pub trigger: Option<String>,
}

/// 已收到、正在显示的一组候选。
#[derive(Debug, Clone)]
struct Shown {
    /// 收到时的上下文：文档、版本、光标与词首。
    context: Context,
    /// 全部候选，按服务器给的顺序。
    items: Vec<CompletionItem>,
    /// 按当前前缀筛选后的序号（`items` 中的下标），顺序与显示顺序一致。
    rows: Vec<usize>,
    /// 选中项在 `rows` 中的位置。
    selected: usize,
    /// 列表显示的第一行（项数多于可见行数时可以滚动）。
    first: usize,
}

/// 补全会话的状态。
#[derive(Default)]
pub(crate) struct Completion {
    /// 上一次看到的上下文，用于判断「是不是在输入」。
    seen: Option<Context>,
    /// 等待发出的请求：上下文与到期的时刻。
    pending: Option<(Context, Instant)>,
    /// 已发出、还没回来的请求：id 与当时的上下文。
    inflight: Option<(u64, Context)>,
    /// 正在显示的候选。
    shown: Option<Shown>,
    /// 按 Esc 关掉的位置（文档，词首）：光标离开这个词之前不再自动弹。
    dismissed: Option<(DocumentId, usize)>,
}

impl Completion {
    /// 返回文档当前光标处的补全上下文；不在可补全的位置时返回 `None`。
    ///
    /// 光标前是标识符字符时，词从该标识符的开头起；紧接一个触发字符（`.`）时词为空，按
    /// 触发字符请求。光标前是空白或行首时不补全；有选区、多个光标时也不补全。
    pub(crate) fn context(doc: &Document) -> Option<Context> {
        if !doc.selection.is_empty() || !doc.extra_selections().is_empty() {
            return None;
        }
        let text = doc.text();
        let caret = doc.selection.head.min(text.end_pos());
        let line = text.line_of(caret);
        let line_start = text.line_start(line);
        let before = text.substring(line_start, caret);
        let len = before.chars().count();
        let mut start = len;
        let mut trigger = None;
        for c in before.chars().rev() {
            if is_identifier(c) {
                start -= 1;
                continue;
            }
            if is_trigger(c) && start == len {
                trigger = Some(c.to_string());
            }
            break;
        }
        let prefix: String = before.chars().skip(start).collect();
        // 既没有已输入的词，也没有触发字符：空白或行首。
        if prefix.is_empty() && trigger.is_none() {
            return None;
        }
        Some(Context {
            doc: doc.id,
            revision: doc.revision,
            caret,
            start: line_start + start,
            prefix,
            trigger,
        })
    }

    /// 每帧看一眼光标处的上下文：开始新的等待、收窄已显示的列表，或关闭列表。
    pub(crate) fn observe(&mut self, context: Option<Context>, now: Instant) {
        let Some(context) = context else {
            self.pending = None;
            self.shown = None;
            self.seen = None;
            return;
        };
        let typed = self
            .seen
            .as_ref()
            .is_some_and(|seen| seen.doc == context.doc && seen.revision != context.revision);
        self.seen = Some(context.clone());
        if self
            .dismissed
            .is_some_and(|(doc, start)| doc == context.doc && start == context.start)
        {
            self.pending = None;
            self.shown = None;
            return;
        }
        // 已显示的列表：同一个词里按新前缀筛选；光标移走（版本没变）或离开这个词时关闭。
        if let Some(shown) = &mut self.shown {
            let same_word = shown.context.doc == context.doc
                && shown.context.start == context.start
                && context.caret >= shown.context.caret
                && (context.revision != shown.context.revision
                    || context.caret == shown.context.caret);
            if same_word {
                shown.filter(&context.prefix);
                if shown.rows.is_empty() {
                    self.shown = None;
                }
                return;
            }
            self.shown = None;
        }
        // 只有输入（同一文档的版本变了）才自动弹；移动光标不弹。
        if !typed {
            return;
        }
        self.pending = Some((context, now + DELAY));
    }

    /// 到期的请求；没有到期时为 `None`。
    pub(crate) fn tick(&mut self, now: Instant) -> Option<Context> {
        let (context, due) = self.pending.clone()?;
        if now < due {
            return None;
        }
        self.pending = None;
        Some(context)
    }

    /// 记下已发出的请求：响应回来时按 `id` 对上。
    pub(crate) fn sent(&mut self, id: u64, context: Context) {
        self.inflight = Some((id, context));
    }

    /// 收到响应：`id` 对不上已发出的请求、文档或版本已变（过期）、或空结果时丢弃。
    ///
    /// 返回是否显示了列表。
    pub(crate) fn response(
        &mut self,
        id: u64,
        context: Option<&Context>,
        items: Vec<CompletionItem>,
    ) -> bool {
        let Some((expected, _)) = &self.inflight else {
            return false;
        };
        // 不是当前请求的响应：可能是一个更早的请求晚到的结果。保留等待状态，让新请求
        // 自己的响应还能对上；否则连发两个请求时，先到的旧响应会把新请求一起作废。
        if *expected != id {
            return false;
        }
        let Some((_, sent)) = self.inflight.take() else {
            return false;
        };
        if context != Some(&sent) || items.is_empty() {
            return false;
        }
        let items: Vec<CompletionItem> = items.into_iter().take(LIMIT).collect();
        self.shown = Some(Shown {
            rows: (0..items.len()).collect(),
            items,
            context: sent,
            selected: 0,
            first: 0,
        });
        true
    }

    /// 接受了一项：不再因这次插入本身重新弹出；之后继续输入或删除照常触发（同 `VSCode`，
    /// 长久的抑制只属于显式按 Esc）。
    pub(crate) fn accepted(&mut self, context: Option<Context>) {
        self.pending = None;
        self.inflight = None;
        self.seen = context;
    }

    /// 按 Esc 关掉列表：本词在光标离开之前不再自动弹。
    pub(crate) fn escaped(&mut self) {
        if let Some(shown) = self.shown.take() {
            self.dismissed = Some((shown.context.doc, shown.context.start));
        } else if let Some((context, _)) = &self.pending {
            self.dismissed = Some((context.doc, context.start));
        }
        self.pending = None;
        self.inflight = None;
    }

    /// 用户换了文档、文档关闭或服务器断开：清空会话。
    pub(crate) fn reset(&mut self) {
        *self = Completion::default();
    }

    /// 列表是否正在显示。
    pub(crate) fn shown(&self) -> bool {
        self.shown.is_some()
    }

    /// 当前显示的候选（`items` 中的下标）。
    pub(crate) fn rows(&self) -> &[usize] {
        self.shown.as_ref().map_or(&[], |s| s.rows.as_slice())
    }

    /// 当前显示的全部候选。
    pub(crate) fn items(&self) -> &[CompletionItem] {
        self.shown.as_ref().map_or(&[], |s| s.items.as_slice())
    }

    /// 选中项在 `rows` 中的位置。
    pub(crate) fn selected(&self) -> usize {
        self.shown.as_ref().map_or(0, |s| s.selected)
    }

    /// 列表显示的第一行。
    pub(crate) fn first(&self) -> usize {
        self.shown.as_ref().map_or(0, |s| s.first)
    }

    /// 高亮 `rows` 中的第 `selected` 项，并把列表滚动到该项可见。
    pub(crate) fn select(&mut self, selected: usize, visible: usize) {
        let Some(shown) = &mut self.shown else {
            return;
        };
        let n = shown.rows.len();
        if n == 0 {
            return;
        }
        shown.selected = selected % n;
        let visible = visible.max(1).min(n);
        if shown.selected < shown.first {
            shown.first = shown.selected;
        } else if shown.selected >= shown.first + visible {
            shown.first = shown.selected + 1 - visible;
        }
        shown.first = shown.first.min(n - visible);
    }

    /// 把列表的第一行设为 `first`（滚轮滚动）。
    pub(crate) fn scroll_to(&mut self, first: usize, visible: usize) {
        let Some(shown) = &mut self.shown else {
            return;
        };
        let n = shown.rows.len();
        shown.first = first.min(n.saturating_sub(visible.max(1)));
    }

    /// 选出当前高亮的候选（`items` 中的下标）与它的上下文，并关闭列表。
    pub(crate) fn take_selected(&mut self) -> Option<(CompletionItem, Context)> {
        let shown = self.shown.take()?;
        let index = *shown.rows.get(shown.selected)?;
        Some((shown.items[index].clone(), shown.context))
    }

    /// 下一次需要检查的时刻（防抖的到期时刻）。
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, due)| *due)
    }
}

impl Shown {
    /// 按前缀筛选候选；选中项若仍在结果里则保持，否则回到第一项。
    fn filter(&mut self, prefix: &str) {
        let current = self.rows.get(self.selected).copied();
        self.rows = (0..self.items.len())
            .filter(|&i| matches_prefix(&self.items[i], prefix))
            .collect();
        self.selected = current
            .and_then(|i| self.rows.iter().position(|&r| r == i))
            .unwrap_or(0);
        let n = self.rows.len();
        self.first = self.first.min(n.saturating_sub(1));
    }
}

/// 候选是否以 `prefix` 开头（比较 `filterText`，没有时用 `label`；不区分大小写）。
fn matches_prefix(item: &CompletionItem, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let text = item.filter_text.as_deref().unwrap_or(&item.label);
    text.to_lowercase().starts_with(&prefix.to_lowercase())
}

/// 一次接受候选时的替换：把文档 `start..end` 换成 `text`，光标默认落在 `text` 末尾。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// 替换范围的起点（字符偏移）。
    pub start: usize,
    /// 替换范围的终点（字符偏移，不含）。
    pub end: usize,
    /// 替换后的文本。
    pub text: String,
    /// 光标在 `text` 内的位置（字符偏移）；`None` 为末尾（函数候选补括号后落在括号之间）。
    pub caret: Option<usize>,
}

/// 返回接受 `item` 时应用的替换：优先按服务器给的 `textEdit` 范围，没有时替换已输入的词。
///
/// 函数、方法、构造候选（LSP 的 kind 2–4）补上调用括号并把光标放到括号之间——gopls 只在
/// 客户端声明 snippet 支持时才回带括号的插入文本，这里不解析 snippet，按种类补（与接受
/// gopls 候选时 `VSCode` 的可见行为一致）；文本已带 `(` 的不重复加。
pub(crate) fn edit_for(doc: &Document, context: &Context, item: &CompletionItem) -> Edit {
    let mut edit = if let Some(edit) = &item.text_edit {
        let start = char_offset(doc, edit.range.start);
        let end = char_offset(doc, edit.range.end).max(start);
        Edit { start, end, text: edit.new_text.clone(), caret: None }
    } else {
        Edit {
            start: context.start,
            end: context.caret,
            text: insertion_text(item),
            caret: None,
        }
    };
    if is_function_kind(item.kind) && !edit.text.contains('(') {
        edit.text.push_str("()");
        edit.caret = Some(edit.text.chars().count() - 1);
    }
    edit
}

/// 候选是否是函数一类（会被调用的）：方法、函数、构造。
fn is_function_kind(kind: Option<u32>) -> bool {
    matches!(kind.unwrap_or(0), 2..=4)
}

/// LSP 位置换算成字符偏移；行、列越界时夹到文本末尾。
fn char_offset(doc: &Document, at: winc_lsp::message::Position) -> usize {
    let text = doc.text();
    let line = (at.line as usize).min(text.line_count().saturating_sub(1));
    let start = text.line_start(line);
    let content = text.line_content(line).slice().to_string();
    start + winc_lsp::position::char_in_line(at, &content).min(text.line_len(line))
}

/// 候选插入的文本：`insert_text` 优先，其次 `label`（`textEdit` 由 [`edit_for`] 处理）。
fn insertion_text(item: &CompletionItem) -> String {
    item.insert_text
        .clone()
        .or_else(|| item.text_edit.as_ref().map(|e| e.new_text.clone()))
        .unwrap_or_else(|| item.label.clone())
}

/// 把一次替换作为单个事务应用到文档：整段插入是一个撤销单元。
pub(crate) fn apply_edit(doc: &mut Document, edit: &Edit) {
    let before = doc.selection;
    let start = edit.start.min(doc.text().end_pos());
    let end = edit.end.min(doc.text().end_pos()).max(start);
    let removed: String = doc.text().substring(start, end);
    let change = winc_core::edit::Change {
        start,
        removed,
        inserted: edit.text.clone(),
    };
    let caret = start
        + edit
            .caret
            .unwrap_or_else(|| edit.text.chars().count());
    doc.apply(winc_core::edit::Transaction::new(
        vec![change],
        before,
        winc_core::Selection::point(caret),
        winc_core::edit::EditKind::Other,
    ));
}

/// 是否是标识符字符：字母、数字、下划线，以及非 ASCII 的字母（Go 允许 Unicode 标识符）。
pub(crate) fn is_identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 是否是成员访问一类的触发字符。
pub(crate) fn is_trigger(c: char) -> bool {
    c == '.'
}

#[cfg(test)]
mod tests;
