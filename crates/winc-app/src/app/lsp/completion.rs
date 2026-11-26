//! 补全的请求与响应：报文的发出、响应的归位，以及候选列表的键盘与鼠标交互。
//!
//! 会话状态（触发、防抖、过滤）在 [`crate::app::editor::completion`]，这里只做「请求怎么发、
//! 响应怎么交回去、列表开着时怎么选」；接受后写进文档也在这里（`accept_completion`）。

use super::*;

impl App {
    /// 补全响应：交给补全会话，过期或对不上的丢弃。
    pub(in crate::app) fn completion_response(&mut self, id: &Id, result: &Json) -> bool {
        let Id::Number(id) = id else {
            return false;
        };
        let Ok(id) = u64::try_from(*id) else {
            return false;
        };
        let response = CompletionResponse::from_json(result);
        let items = response.map(|r| r.items().to_vec()).unwrap_or_default();
        let current = self
            .editor
            .active()
            .and_then(editor::completion::Completion::context);
        self.completion.response(id, current.as_ref(), items)
    }

    /// 发出补全请求；文档没同步给服务器时丢弃。
    pub(super) fn send_completion(&mut self, context: &editor::completion::Context) {
        let Some(synced) = self.lsp.synced().get(&context.doc.0) else {
            return;
        };
        let Some(doc) = self.editor.document(context.doc) else {
            return;
        };
        let params = CompletionParams {
            uri: synced.uri.clone(),
            position: position_of(doc, context.caret),
            context: Some(CompletionContext {
                trigger_kind: if context.trigger.is_some() { 2 } else { 1 },
                trigger_character: context.trigger.clone(),
            }),
        };
        let id = self.lsp.next_id();
        self.completion.sent(id, context.clone());
        self.send_lsp(&Message::Request {
            id: Id::Number(id as i64),
            method: method::COMPLETION.into(),
            params: Some(params.to_json()),
        });
    }

    /// 补全列表开着时处理 ↑↓（换选中项）、Enter/Tab（接受）、Esc（关闭）；返回是否处理了
    /// 这个键。输入法组字、Vim 不在插入模式、焦点不在编辑区时不接管。
    pub(in crate::app) fn completion_key(&mut self, key: &Key) -> bool {
        if self.focus != Focus::Editor
            || !self.completion.shown()
            || !self.vim_typing()
            || !self.preedit_at(Focus::Editor).is_empty()
        {
            return false;
        }
        let m = self.pointer.modifiers;
        if m.shift_key() || m.control_key() || m.alt_key() || m.super_key() {
            return false;
        }
        let n = self.completion.rows().len();
        if n == 0 {
            return false;
        }
        match key {
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let down = matches!(key, Key::Named(NamedKey::ArrowDown));
                let selected = self.completion.selected();
                let step = if down { 1 } else { n - 1 };
                self.completion
                    .select((selected + step) % n, COMPLETION_VISIBLE);
            }
            Key::Named(NamedKey::Enter | NamedKey::Tab) => self.accept_completion(),
            Key::Named(NamedKey::Escape) => self.completion.escaped(),
            _ => return false,
        }
        self.request_redraw();
        true
    }

    /// 鼠标点选补全列表的第 `row` 行：高亮它并接受。
    pub(in crate::app) fn click_completion(&mut self, row: usize) {
        self.completion.select(row, COMPLETION_VISIBLE);
        self.accept_completion();
    }

    /// 鼠标在补全列表上时按滚轮位移 `dy` 滚动列表，返回是否已处理。
    pub(in crate::app) fn scroll_completion(&mut self, (x, y): (f32, f32), dy: f32) -> bool {
        let over = self
            .hits
            .completion_card
            .is_some_and(|card| card.contains(x, y));
        if !over {
            return false;
        }
        let n = self.completion.rows().len();
        if n <= COMPLETION_VISIBLE {
            return true;
        }
        let Some(row) = self
            .hits
            .completion
            .first()
            .map(|(r, _)| r.h)
            .filter(|h| *h > 0.0)
        else {
            return true;
        };
        // 累积不足一行的位移：触控板的小步滚动也能平滑滚到下一行。
        let steps = (dy / row).trunc();
        let first = self.completion.first() as isize - steps as isize;
        self.completion.scroll_to(
            first.clamp(0, (n - COMPLETION_VISIBLE) as isize) as usize,
            COMPLETION_VISIBLE,
        );
        self.request_redraw();
        true
    }

    /// 接受当前高亮的候选：按服务器的 `textEdit` 或已输入的词替换，整段插入是一个撤销单元。
    pub(in crate::app) fn accept_completion(&mut self) {
        let Some((item, context)) = self.completion.take_selected() else {
            return;
        };
        if self.editor.active().map(|d| d.id) != Some(context.doc) {
            return;
        }
        let Some(doc) = self.editor.active() else {
            return;
        };
        let edit = editor::completion::edit_for(doc, &context, &item);
        let lines_before = self.editor.active().map_or(0, |d| d.text().line_count());
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        editor::completion::apply_edit(doc, &edit);
        let current = self
            .editor
            .active()
            .and_then(editor::completion::Completion::context);
        self.completion.accepted(current);
        let lines_after = self.editor.active().map_or(0, |d| d.text().line_count());
        if lines_after != lines_before {
            self.relayout();
        }
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 每帧：按光标处的上下文推进补全会话。输入法组字期间不看不改（按键交给输入法）。
    pub(in crate::app) fn track_completion(&mut self) {
        let now = Instant::now();
        if !self.preedit_at(Focus::Editor).is_empty() {
            return;
        }
        if self.focus != Focus::Editor {
            self.completion.observe(None, now);
            return;
        }
        let enabled = self.vim_typing() && self.lsp.ready();
        let context = enabled
            .then(|| {
                self.editor
                    .active()
                    .filter(|doc| self.lsp.is_synced(doc.id))
                    .and_then(editor::completion::Completion::context)
            })
            .flatten();
        self.completion.observe(context, now);
    }
}
