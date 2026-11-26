//! 名字旁边的提示气泡：转到定义没有找到定义、未找到其他用法、光标处没有可以查找的名字时，
//! 在光标所在的名字上方弹出一句话，而不是写在状态栏里。
//!
//! 气泡挂在文档中的位置上，随文档滚动；停留 [`HINT_DURATION`] 后淡出，按键、点击、滚动或
//! 打开浮窗时立即淡出。

use std::time::{Duration, Instant};

use winc_core::movement;
use winc_editor::DocumentId;
use winc_render::Rect;

use super::*;

/// 提示气泡停留的时长：够读完一句话。
const HINT_DURATION: Duration = Duration::from_millis(3000);
/// 诊断气泡停留的时长：消息往往比一句话长，多给一点时间读完。
const DIAGNOSTIC_DURATION: Duration = Duration::from_millis(5000);

/// 名字旁边的提示气泡。
pub struct NameHint {
    /// 提示文字。
    pub text: String,
    /// 气泡挂在哪个文档上。
    pub doc: DocumentId,
    /// 气泡挂在文档中的位置（字符偏移）：光标所在名字的开头。
    pub anchor: usize,
    /// 到这一刻开始淡出。
    until: Instant,
    /// 出现的动画；结束后为 `None`。
    pub fade_in: Option<Clock>,
    /// 消失的动画；还没开始消失时为 `None`。
    pub fade_out: Option<Clock>,
    /// 是光标处诊断消息的气泡（不是名字旁的提示）：离开诊断范围时由每帧的检查淡出。
    pub diagnostic: bool,
    /// 诊断气泡：这批消息的最高严重级别（0 = 错误，1 = 警告），决定左侧图标。
    pub level: Option<u32>,
}

/// 诊断气泡里的一行文字：位置、内容与每个字符的右边界，供拖选与复制时按位置找字。
#[derive(Debug, Clone)]
pub(crate) struct HintLine {
    /// 这一行占用的整行矩形；选区的底色按其高度绘制。
    pub row: Rect,
    /// 第一个字符的左边界（逻辑像素）。
    pub x: f32,
    /// 绘制的文字。
    pub text: String,
    /// 每个字符的右边界（逻辑像素），与 `text` 的字符一一对应。
    pub edges: Vec<f32>,
}

impl HintLine {
    /// 返回横坐标 `x` 之后的第一个字符的序号（0 到字符数）；越过字符宽度的一半即算在其后。
    fn index_at(&self, x: f32) -> usize {
        let mut left = self.x;
        for (i, &right) in self.edges.iter().enumerate() {
            if x < (left + right) / 2.0 {
                return i;
            }
            left = right;
        }
        self.edges.len()
    }
}

/// 诊断气泡中文字选区的一端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HintPoint {
    /// 所在行（`Hits::hint_lines` 的下标）。
    pub line: usize,
    /// 行内的字符序号。
    pub ch: usize,
}

impl App {
    /// 在 `anchor` 上方弹出诊断消息 `text`：鼠标停在诊断范围内时调用，离开范围后淡出。
    ///
    /// 与 [`App::hint_at_name`] 共用同一个气泡，多出停顿时长：消息通常更长。
    pub(in crate::app) fn hint_at_diagnostic(
        &mut self,
        text: impl Into<String>,
        anchor: usize,
        level: u32,
    ) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (text, id) = (text.into(), doc.id);
        let fade_in = self.clock(POPUP_IN);
        self.jumps.hint_selection = None;
        self.jumps.name_hint = Some(NameHint {
            text,
            doc: id,
            anchor,
            until: Instant::now() + DIAGNOSTIC_DURATION,
            fade_in,
            fade_out: None,
            diagnostic: true,
            level: Some(level),
        });
        self.request_redraw();
    }

    /// 鼠标仍停在诊断上：把淡出时刻往后推，停多久就看多久。
    pub(in crate::app) fn hold_diagnostic_hint(&mut self) {
        let Some(hint) = &mut self.jumps.name_hint else {
            return;
        };
        if hint.diagnostic && hint.fade_out.is_none() {
            hint.until = Instant::now() + DIAGNOSTIC_DURATION;
        }
    }

    /// 当前光标处的诊断气泡是否已经显示 `text`（同一个范围的同一条消息不重复弹出）。
    pub(in crate::app) fn diagnostic_hint_is(&self, text: &str, anchor: usize) -> bool {
        self.jumps
            .name_hint
            .as_ref()
            .is_some_and(|h| h.text == text && h.anchor == anchor && h.fade_out.is_none())
    }

    /// 气泡里 `(x, y)` 处的字的位置。
    ///
    /// `near` 为 `true` 时（拖着选的时候）不在任何一行上也取最近的一行，拖到首行以上或末行
    /// 以下时停在首尾；否则只认正落在某一行上的位置。
    pub(in crate::app) fn hint_text_at(&self, x: f32, y: f32, near: bool) -> Option<HintPoint> {
        let lines = &self.hits.hint_lines;
        // 同一行上可能有几段（多行消息的每一行是一段）：取开头在鼠标左边的最后一段。
        let on = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.row.y <= y && y < l.row.bottom())
            .filter(|(_, l)| l.x - 2.0 <= x)
            .map(|(i, _)| i)
            .next_back()
            .or_else(|| {
                lines
                    .iter()
                    .position(|l| l.row.y <= y && y < l.row.bottom())
                    .filter(|_| near)
            });
        let line = match on {
            Some(i) => i,
            None if near => {
                let first = lines.first()?;
                if y < first.row.y {
                    return Some(HintPoint { line: 0, ch: 0 });
                }
                let last = lines.len() - 1;
                if y >= lines[last].row.bottom() {
                    return Some(HintPoint {
                        line: last,
                        ch: lines[last].edges.len(),
                    });
                }
                // 两行之间的空隙：算上面那一行的行尾。
                let above = lines.iter().rposition(|l| l.row.bottom() <= y)?;
                return Some(HintPoint {
                    line: above,
                    ch: lines[above].edges.len(),
                });
            }
            None => return None,
        };
        // 只在字上（不在行尾以外的空白里）才算按在字上；拖着的时候行尾以外算到行尾。
        let l = &lines[line];
        let end = l.edges.last().copied().unwrap_or(l.x);
        if !near && (x < l.x - 2.0 || x > end + 4.0) {
            return None;
        }
        Some(HintPoint {
            line,
            ch: l.index_at(x),
        })
    }

    /// 气泡里第 `line` 行选中了第几个字到第几个字（`len` 是这一行的字数）；这一行没选中时
    /// 为 `None`。
    pub(in crate::app) fn hint_selected_chars(
        &self,
        line: usize,
        len: usize,
    ) -> Option<(usize, usize)> {
        let (a, b) = self.jumps.hint_selection?;
        let (from, to) = (a.min(b), a.max(b));
        if line < from.line || line > to.line {
            return None;
        }
        let start = if line == from.line {
            from.ch.min(len)
        } else {
            0
        };
        let end = if line == to.line { to.ch.min(len) } else { len };
        (start < end).then_some((start, end))
    }

    /// 气泡里选中的文字；没选中时为 `None`。
    ///
    /// 折成多行的消息按屏幕上的各行以换行符连接。
    pub(in crate::app) fn hint_selected_text(&self) -> Option<String> {
        let (a, b) = self.jumps.hint_selection?;
        let (from, to) = (a.min(b), a.max(b));
        let mut out = String::new();
        for line in from.line..=to.line.min(self.hits.hint_lines.len().saturating_sub(1)) {
            let l = self.hits.hint_lines.get(line)?;
            let len = l.edges.len();
            let (start, end) = self.hint_selected_chars(line, len).unwrap_or((0, 0));
            if line > from.line {
                out.push('\n');
            }
            out.extend(l.text.chars().skip(start).take(end - start));
        }
        (!out.trim().is_empty()).then_some(out)
    }

    /// 气泡里是否有选中的文字（此时按复制键要复制气泡而不是编辑区）。
    pub(in crate::app) fn hint_has_selection(&self) -> bool {
        self.jumps.hint_selection.is_some()
    }

    /// 在气泡的文字上按下鼠标：按在字上开始选字，`count` 为 2（双击）时选中这一整行；按在
    /// 空白处清掉选中。返回是否继续拖动以扩展选区。
    pub(in crate::app) fn press_hint_text(&mut self, x: f32, y: f32, count: u8) -> bool {
        self.focus = Focus::Editor;
        let Some(at) = self.hint_text_at(x, y, false) else {
            self.jumps.hint_selection = None;
            self.request_redraw();
            return false;
        };
        self.jumps.hint_selection = Some(if count >= 2 {
            let len = self
                .hits
                .hint_lines
                .get(at.line)
                .map_or(0, |l| l.edges.len());
            (
                HintPoint {
                    line: at.line,
                    ch: 0,
                },
                HintPoint {
                    line: at.line,
                    ch: len,
                },
            )
        } else {
            (at, at)
        });
        self.request_redraw();
        count < 2
    }

    /// 在气泡里拖着选字时鼠标到了 `(x, y)`。
    pub(in crate::app) fn drag_hint_text(&mut self, x: f32, y: f32) {
        if let (Some((anchor, _)), Some(head)) =
            (self.jumps.hint_selection, self.hint_text_at(x, y, true))
        {
            self.jumps.hint_selection = Some((anchor, head));
            self.request_redraw();
        }
    }

    /// 在光标所在的名字上方弹出提示 `text`；没有打开文档时改为写在状态栏。
    pub(super) fn hint_at_name(&mut self, text: impl Into<String>) {
        let text = text.into();
        let Some(doc) = self.editor.active() else {
            self.flash(text);
            return;
        };
        let anchor = self.name_span().map_or_else(
            || movement::word_at(doc.text(), doc.selection.head).0,
            |s| s.0,
        );
        let id = doc.id;
        let fade_in = self.clock(POPUP_IN);
        self.jumps.hint_selection = None;
        self.jumps.name_hint = Some(NameHint {
            text,
            doc: id,
            anchor,
            until: Instant::now() + HINT_DURATION,
            fade_in,
            fade_out: None,
            diagnostic: false,
            level: None,
        });
        self.request_redraw();
    }

    /// 让提示气泡淡出；没有窗口（测试中）或动态效果关闭时直接去掉。
    pub(in crate::app) fn dismiss_hint(&mut self) {
        // 气泡要走之前先放掉选中的文字：复制走的是同一把气泡，见 `App::copy`。
        self.jumps.hint_selection = None;
        let Some(hint) = &self.jumps.name_hint else {
            return;
        };
        if hint.fade_out.is_some() {
            return;
        }
        match self.clock(POPUP_OUT) {
            Some(clock) => {
                if let Some(hint) = &mut self.jumps.name_hint {
                    hint.fade_out = Some(clock);
                }
            }
            None => self.jumps.name_hint = None,
        }
        self.request_redraw();
    }

    /// 停留时间到了就开始淡出；返回下一次需要检查的时刻。
    pub(in crate::app) fn hint_tick(&mut self, now: Instant) -> Option<Instant> {
        let hint = self.jumps.name_hint.as_ref()?;
        if hint.fade_out.is_some() {
            return None;
        }
        if now >= hint.until {
            self.dismiss_hint();
            None
        } else {
            Some(hint.until)
        }
    }

    /// 清除已经结束的出现、消失动画；消失结束后去掉气泡。
    pub(in crate::app) fn settle_hint(&mut self) {
        let Some(hint) = &mut self.jumps.name_hint else {
            return;
        };
        if hint.fade_in.is_some_and(|c| c.done()) {
            hint.fade_in = None;
        }
        if hint.fade_out.is_some_and(|c| c.done()) {
            self.jumps.name_hint = None;
            self.jumps.hint_selection = None;
        }
    }

    /// 提示气泡是否在动画中。
    pub(in crate::app) fn hint_animating(&self) -> bool {
        self.jumps
            .name_hint
            .as_ref()
            .is_some_and(|h| h.fade_in.is_some() || h.fade_out.is_some())
    }
}
