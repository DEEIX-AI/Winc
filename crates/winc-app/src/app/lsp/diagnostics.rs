//! 诊断的接收与展示：gopls 推送的诊断入库、按行分组的波浪线取色，以及鼠标悬停的消息气泡。

use super::*;

impl App {
    /// 诊断推送：整表替换该文档的诊断并重绘。
    pub(in crate::app) fn lsp_diagnostics(&mut self, params: PublishDiagnosticsParams) {
        let Some(path) = message::uri_to_path(&params.uri) else {
            return;
        };
        let Some(doc) = self
            .editor
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path.as_path()))
        else {
            return;
        };
        let id = doc.id;
        let problems = editor::diagnostics::convert(doc, &params.diagnostics);
        self.diagnostics.replace(id, problems);
        self.request_redraw();
    }

    /// 文档 `id` 的诊断按行分组，颜色取自主题：错误与警告两档。
    ///
    /// 只处理可见范围由绘制方决定；这里返回整个文档的分组（大文档的诊断本身不多）。
    pub(crate) fn diagnostic_lines(
        &self,
        id: DocumentId,
    ) -> Vec<(usize, Vec<editor::diagnostics::Underline>)> {
        let Some(doc) = self.editor.document(id) else {
            return Vec::new();
        };
        let p = self.palette;
        (self.diagnostics.by_line(doc).into_iter())
            .map(|(line, rows)| {
                let rows = rows
                    .into_iter()
                    .map(|(start, end, severity)| editor::diagnostics::Underline {
                        start,
                        end,
                        color: match severity {
                            winc_lsp::message::Severity::Error => p.error,
                            winc_lsp::message::Severity::Warning => p.warning,
                        },
                    })
                    .collect();
                (line, rows)
            })
            .collect()
    }

    /// 鼠标移动到诊断波浪线上停稳后弹出消息气泡，移开后淡出。
    ///
    /// 悬停只由**鼠标移动**触发（同 `VSCode`）：位置变化才算一次移动，移到某处才开始待发，
    /// 满 [`HOVER_STILL`] 时只评估一次——当时鼠标下有诊断就弹，没有就作废。因此打字时出现
    /// 在静止鼠标下的新诊断不弹，打字停顿后也不弹；任意按键作废待发，补全列表显示期间不弹。
    /// 按严重级别从高到低合并同一处的多条诊断；鼠标停着不动时保持气泡，不自动超时消失。
    ///
    /// 鼠标移动事件里也会直接调用（见 `App::on_mouse_move`）：普通的光标移动不一定让界面
    /// 重绘，只在每帧的 `prepare_frame` 里计时会漏掉最后的移动——鼠标停稳后没有新事件，
    /// 计时就永远没开始，气泡时有时无。事件里先记下起算时刻，`tick` 的到期唤醒负责评估。
    pub(in crate::app) fn track_diagnostic_hint(&mut self, now: Instant) {
        let Some(doc) = self.editor.active() else {
            self.end_diagnostic_hint();
            return;
        };
        // 补全列表打开时不弹诊断气泡（VSCode 的建议列表显示时同样隐藏悬停）。
        if self.completion.shown() {
            self.end_diagnostic_hint();
            return;
        }
        // 与上一次记录的鼠标位置不同才算一次「鼠标移动」：重新开始悬停计时。
        let (x, y) = self.pointer.mouse;
        let moved = self.lsp.hover_pos != Some((x, y));
        self.lsp.hover_pos = Some((x, y));
        if moved {
            self.lsp.hover_at = Some(((x, y), now));
        }
        // 鼠标已移到气泡上（卡片或指向文字的小三角），或正在气泡里拖选：保持气泡，不因离开
        // 波浪线范围而淡出，这样才够得着、选得中消息文字。
        if self.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic)
            && (self
                .hits
                .hint_hover
                .is_some_and(|hover| hover.contains(x, y))
                || matches!(self.pointer.drag, Drag::HintText))
        {
            self.lsp.disarm_diagnostic_hint();
            self.hold_diagnostic_hint();
            return;
        }
        if !doc.shows_text() || !self.pointer.cursor_inside || !self.over_editor_text() {
            self.end_diagnostic_hint();
            return;
        }
        let id = doc.id;
        let Some(pos) = self.pos_at(x, y) else {
            self.end_diagnostic_hint();
            return;
        }; // 先在只读借用里算出要显示的内容，之后才能改界面状态。
        let found = {
            let mut hits = self.diagnostics.at(id, pos);
            if hits.is_empty() {
                None
            } else {
                let anchor = hits.iter().map(|p| p.start).min().unwrap_or(0);
                hits.sort_by_key(|p| match p.severity {
                    winc_lsp::message::Severity::Error => 0,
                    winc_lsp::message::Severity::Warning => 1,
                });
                let text = hits
                    .iter()
                    .map(|p| p.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                // 排序后首条即最高级别（错误 0，警告 1）。
                let level = hits.first().map_or(0, |p| match p.severity {
                    winc_lsp::message::Severity::Error => 0,
                    winc_lsp::message::Severity::Warning => 1,
                });
                Some((anchor, text, level))
            }
        };
        let Some((anchor, text, level)) = found else {
            // 鼠标仍在文字上但此处没有诊断：旧气泡淡出，待发保留到到期时再看一次。
            self.fade_diagnostic_hint();
            return;
        };
        // 同一处同样的消息：鼠标停着就一直显示（待发随之作废，避免反复报到期的唤醒）。
        if self.diagnostics_hint_seen(id, anchor, &text) {
            self.lsp.disarm_diagnostic_hint();
            self.hold_diagnostic_hint();
            return;
        }
        // 唯一的弹出路径：鼠标移动过来并停满 HOVER_STILL。到点只看一次，没赶上就作废；
        // 其余情形（按键后、静止鼠标下的新诊断）都只淡出。
        match self.lsp.hover_at {
            Some((at, _)) if at != (x, y) => {
                self.lsp.disarm_diagnostic_hint();
                self.end_diagnostic_hint();
            }
            Some((_, since)) if now.duration_since(since) >= HOVER_STILL => {
                self.lsp.disarm_diagnostic_hint();
                self.lsp.remember_diag_hint(id, anchor, &text);
                self.hint_at_diagnostic(text, anchor, level);
            }
            // 还没停稳：保持待发，由 `hover_tick` 在到期时唤醒评估；旧气泡先淡出。
            Some(_) => self.fade_diagnostic_hint(),
            None => self.end_diagnostic_hint(),
        }
    }

    /// 悬停待发的到期检查：到点就地评估一次，不等下一帧（鼠标停下后没有新事件）。
    ///
    /// 返回下一次需要检查的时刻；鼠标不在编辑区文字上、按键后或评估完毕时为 `None`。
    pub(in crate::app) fn hover_tick(&mut self, now: Instant) -> Option<Instant> {
        let at = self.lsp.hover_deadline()?;
        if now < at {
            return Some(at);
        }
        self.track_diagnostic_hint(now);
        self.lsp.hover_deadline()
    }

    /// 鼠标不在诊断上：诊断气泡淡出、清掉弹出记忆，悬停待发一并作废。
    fn end_diagnostic_hint(&mut self) {
        self.lsp.disarm_diagnostic_hint();
        self.fade_diagnostic_hint();
    }

    /// 气泡淡出并清掉弹出记忆；不动悬停待发。
    fn fade_diagnostic_hint(&mut self) {
        self.lsp.forget_diag_hint();
        if self.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic) {
            self.dismiss_hint();
        }
    }

    /// 当前光标处的诊断气泡是否已经针对同一处、同样的消息弹出过。
    fn diagnostics_hint_seen(&self, doc: DocumentId, anchor: usize, text: &str) -> bool {
        self.lsp.diag_hint_seen(doc, anchor, text) || self.diagnostic_hint_is(text, anchor)
    }
}
