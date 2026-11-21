//! Properties 预览的只读选区。位置属于解码后的行，不写入源文档的选区。

use ropey::Rope;
use winc_core::{Lines, movement, text};
use winc_editor::{COPY_LIMIT, DocumentId};

use super::{PropertiesPreview, decoded_line, horizontal_track};
use crate::app::{App, Drag};
use crate::layout::drawn_scroll_line;
use crate::theme::TEXT_PAD;

type Position = (usize, usize);

impl PropertiesPreview {
    /// 按先后顺序返回非空选区的两端。
    pub(crate) fn selection(&self) -> Option<(Position, Position)> {
        let (a, b) = self.selection?;
        (a != b).then_some((a.min(b), a.max(b)))
    }
}

impl App {
    /// 选区是否仍属于当前文档的这一版本。
    pub(in crate::app) fn properties_has_selection(&self) -> bool {
        let view = &self.preview.properties;
        self.properties_area().is_some()
            && self
                .editor
                .active()
                .is_some_and(|doc| view.doc == Some(doc.id) && view.revision == Some(doc.revision))
            && view.selection().is_some()
    }

    /// 返回选中的完整解码文字。没有有效选区或超过复制大小上限时返回 `None`。
    ///
    /// 绘制行的省略号不属于内容；读取源文档的完整行，保留实际行尾，包括混合行尾。
    pub(crate) fn properties_selected_text(&self) -> Option<String> {
        if !self.properties_has_selection() {
            return None;
        }
        let doc = self.editor.active()?;
        let ((first, from), (last, to)) = self.preview.properties.selection()?;
        let lines = doc.text().line_count();
        let mut selected = String::new();
        for line in first..=last.min(lines - 1) {
            let span = doc.text().line_span(line);
            let source = doc.text_range(span.start, span.end)?;
            let decoded = winc_preview::properties::decode(&source);
            let start = if line == first { from } else { 0 };
            let end = if line == last { to } else { usize::MAX };
            let byte_at = |col| {
                decoded
                    .char_indices()
                    .nth(col)
                    .map_or(decoded.len(), |(byte, _)| byte)
            };
            let part = &decoded[byte_at(start)..byte_at(end)];
            if selected.len() as u64 + part.len() as u64 > COPY_LIMIT {
                return None;
            }
            selected.push_str(part);
        }
        Some(selected)
    }

    /// 全选用文档末尾标记，复制时不受绘制长行的字符上限影响。
    pub(crate) fn properties_select_all(&mut self) {
        self.prepare_properties();
        if let Some(doc) = self
            .editor
            .active()
            .filter(|_| self.properties_area().is_some())
        {
            self.preview.properties.selection =
                (doc.text().end_pos() > 0).then_some(((0, 0), (doc.text().line_count(), 0)));
            self.request_redraw();
        }
    }

    /// 点击放置选区起点，双击选词，按住 Shift 扩展已有选区。
    pub(in crate::app) fn click_properties(&mut self, x: f32, y: f32) {
        self.focus_properties();
        let Some(at) = self.properties_position_at(x, y) else {
            return;
        };
        let count = self.click_count(x, y);
        let shift = self.pointer.modifiers.shift_key();
        let selection = if count >= 2 {
            let doc = self.editor.active().unwrap();
            if at.0 < doc.text().line_count() {
                let line = Rope::from_str(&decoded_line(doc, at.0));
                let (from, to) = movement::word_at(&line, at.1);
                ((at.0, from), (at.0, to.min(text::MAX_LINE_CHARS)))
            } else {
                (at, at)
            }
        } else if shift {
            (
                self.preview
                    .properties
                    .selection
                    .map_or(at, |(anchor, _)| anchor),
                at,
            )
        } else {
            (at, at)
        };
        self.preview.properties.selection = Some(selection);
        self.pointer.drag = Drag::PropertiesText;
        self.request_redraw();
    }

    /// 拖出预览边缘时滚动，选区始终使用预览自身的坐标。
    pub(in crate::app) fn drag_properties(&mut self, x: f32, y: f32) {
        self.prepare_properties();
        if self.preview.properties.selection.is_none() {
            self.pointer.drag = Drag::None;
            return;
        }
        let Some(area) = self.properties_area().map(horizontal_track) else {
            return;
        };
        if y < area.y {
            self.scroll_properties_by(0.0, (area.y - y).min(60.0));
        } else if y > area.bottom() {
            self.scroll_properties_by(0.0, -(y - area.bottom()).min(60.0));
        }
        if x < area.x {
            self.scroll_properties_by((area.x - x).min(60.0), 0.0);
        } else if x > area.right() {
            self.scroll_properties_by(-(x - area.right()).min(60.0), 0.0);
        }
        self.prepare_properties();
        let x = x.clamp(area.x, area.right().max(area.x));
        let y = y.clamp(area.y, (area.bottom() - 1.0).max(area.y));
        if let Some(at) = self.properties_position_at(x, y)
            && let Some((anchor, _)) = self.preview.properties.selection
        {
            self.preview.properties.selection = Some((anchor, at));
        }
        self.request_redraw();
    }

    /// 右键点击预览：取得焦点，保留点中选区，取消点外选区。
    pub(in crate::app) fn properties_context_target(
        &mut self,
        x: f32,
        y: f32,
    ) -> Option<DocumentId> {
        let covered = self.layout.search.contains(x, y)
            || (self.bars.goto_input.is_some() && self.prompt_bar().contains(x, y))
            || self.over_corner_card(x, y);
        if covered || !self.properties_area_at(x, y) {
            return None;
        }
        self.focus_properties();
        let inside = self.properties_position_at(x, y).is_some_and(|at| {
            self.preview
                .properties
                .selection()
                .is_some_and(|(a, b)| (a..=b).contains(&at))
        });
        if !inside {
            self.preview.properties.selection = None;
        }
        self.request_redraw();
        self.editor.active().map(|doc| doc.id)
    }

    /// 使用与绘制相同的制表位、组合字符和 emoji 宽度，并吸附到字素边界。
    fn properties_position_at(&self, x: f32, y: f32) -> Option<Position> {
        let area = self.properties_ready()?;
        let doc = self.editor.active()?;
        let view = &self.preview.properties;
        let scroll = drawn_scroll_line(view.top_line, self.fonts.line_height);
        let line = (scroll + f64::from((y - area.y) / self.fonts.line_height)).max(0.0) as usize;
        if line >= doc.text().line_count() {
            return Some((doc.text().line_count(), 0));
        }
        let shown = Rope::from_str(&decoded_line(doc, line));
        let cell = ((x - area.x - TEXT_PAD + view.scroll_x) / self.metrics.cell)
            .round()
            .max(0.0) as usize;
        let col = text::cell_to_col(shown.slice(..), cell, self.editor.active_tab_width());
        let col = movement::snap_to_grapheme(&shown, col.min(text::MAX_LINE_CHARS));
        Some((line, col))
    }
}
