//! Properties 预览的位置和输入。滚动按物理行计，与源代码的折叠、自动换行无关。

use std::ops::Range;

use winc_core::Lines;
use winc_core::text::{MAX_LINE_CHARS, text_cells};
use winc_editor::{Document, DocumentId, PreviewMode};
use winc_render::Rect;
use winit::keyboard::{Key, NamedKey};

use crate::app::scroll::editor::top_source_line;
use crate::app::{App, Focus};
use crate::layout::{
    drawn_scroll_line, editor_content_height, editor_max_scroll, hscrollbar_thumb,
    scroll_for_thumb, scrollbar_thumb,
};
use crate::theme::{SCROLLBAR_HIT, TEXT_PAD};

mod selection;

#[derive(Default)]
pub(crate) struct PropertiesPreview {
    doc: Option<DocumentId>,
    revision: Option<u64>,
    /// 解码后的（物理行，字符列）；行数处的位置表示完整文档末尾。
    selection: Option<((usize, usize), (usize, usize))>,
    /// 顶部物理行，可为小数。
    pub(crate) top_line: f64,
    pub(crate) scroll_x: f32,
    pub(crate) max_scroll_x: f32,
    /// 最近同步的源代码位置；源代码再次滚动时才带动预览。
    source_line: Option<f64>,
    pub(in crate::app) focused: bool,
    visible: Option<VisibleLines>,
}

struct VisibleLines {
    revision: u64,
    tab_width: usize,
    range: Range<usize>,
    rows: Vec<Vec<char>>,
    widest: usize,
}

impl PropertiesPreview {
    pub(crate) fn lines(&self) -> impl Iterator<Item = (usize, &[char])> {
        self.visible
            .iter()
            .flat_map(|view| view.range.clone().zip(view.rows.iter().map(Vec::as_slice)))
    }
}

impl App {
    /// 当前 Properties 预览的区域。分栏在空间不足时只显示源代码。
    pub(crate) fn properties_area(&self) -> Option<Rect> {
        let doc = self.editor.active()?;
        if !doc.is_properties() {
            return None;
        }
        let area = match doc.preview_mode() {
            PreviewMode::Source => return None,
            PreviewMode::Split => self.layout.split,
            PreviewMode::Preview => self.layout.editor,
        };
        (area.w > 0.0 && area.h > 0.0).then_some(area)
    }

    /// 位置 (`x`, `y`) 是否落在 Properties 预览中。
    pub(crate) fn properties_area_at(&self, x: f32, y: f32) -> bool {
        self.properties_area()
            .is_some_and(|area| area.contains(x, y))
    }

    /// 只看预览时总是把编辑区的输入交给预览；分栏时由最近点击的一侧决定。
    pub(crate) fn properties_focused(&self) -> bool {
        self.focus == Focus::Editor
            && self.properties_area().is_some()
            && self.editor.active().is_some_and(|doc| {
                doc.preview_mode() == PreviewMode::Preview
                    || (self.preview.properties.doc == Some(doc.id)
                        && self.preview.properties.focused)
            })
    }

    /// 在切换模式前记录原来的顶部行。进入预览时使用当前分栏位置或源代码位置，
    /// 离开预览时在重新布局源代码后恢复对应的行。
    pub(super) fn prepare_properties_mode(&mut self, mode: PreviewMode) -> Option<f64> {
        let doc = self.editor.active()?;
        let old = doc.preview_mode();
        if old == mode || !doc.is_properties() {
            return None;
        }
        self.settle_editor_glide();
        // 分栏可能因窗口太窄而隐藏，这时没有可沿用或可同步的预览位置。
        let pane_visible = self.properties_area().is_some();
        if pane_visible {
            self.prepare_properties();
        }
        let doc = self.editor.active()?;
        let view = &mut self.preview.properties;
        view.focused = false;
        view.selection = None;
        if old == PreviewMode::Preview {
            return Some(view.top_line);
        }
        let top = top_source_line(doc);
        // 从源代码进入分栏或只看预览：右侧跟着左侧当前的阅读位置开始。分栏可见时切到只看
        // 预览沿用分栏已有的位置（上面已经同步过），不重置；分栏因空间不足隐藏时没有可沿用
        // 的位置，同样从源码开始。
        if old == PreviewMode::Source || !pane_visible {
            view.show(doc.id, top);
        }
        view.source_line = Some(top);
        None
    }

    /// 记下预览带动的源代码位置。折叠处的源代码视口可能比预览提前几行。
    pub(super) fn remember_properties_source_line(&mut self) {
        self.preview.properties.source_line = self.editor.active().map(top_source_line);
    }

    /// 将位置限制在当前视口内，缓存本屏解码后的行和字宽，供绘制与鼠标共用。
    pub(crate) fn prepare_properties(&mut self) {
        let (Some(area), Some(doc)) = (self.properties_area(), self.editor.active()) else {
            return;
        };
        let lh = self.fonts.line_height;
        let lines = doc.text().line_count();
        let max = editor_max_scroll(lines, area.h, lh);
        let top = top_source_line(doc);
        let view = &mut self.preview.properties;
        if view.doc != Some(doc.id) {
            view.show(doc.id, top);
        }
        if view.revision != Some(doc.revision) {
            view.revision = Some(doc.revision);
            view.selection = None;
        }
        if view.source_line != Some(top) {
            view.top_line = top;
            view.source_line = Some(top);
        }
        view.top_line = view.top_line.clamp(0.0, max);
        let scroll = drawn_scroll_line(view.top_line, lh);
        let first = scroll.floor() as usize;
        let last = (scroll + f64::from(area.h / lh)).ceil() as usize;
        let range = first..last.min(lines);
        let tab_width = doc.tab_width(self.editor.tab_width);
        if view.visible.as_ref().is_none_or(|visible| {
            visible.revision != doc.revision
                || visible.range != range
                || visible.tab_width != tab_width
        }) {
            let rows: Vec<Vec<char>> = range
                .clone()
                .map(|line| decoded_line(doc, line).chars().collect())
                .collect();
            let widest = rows
                .iter()
                .map(|chars| text_cells(chars.iter().copied(), 0, tab_width))
                .max()
                .unwrap_or(0);
            view.visible = Some(VisibleLines {
                revision: doc.revision,
                tab_width,
                range,
                rows,
                widest,
            });
        }
        let width = view.visible.as_ref().map_or(0, |visible| visible.widest);
        let track = horizontal_track(area);
        view.max_scroll_x = (width as f32 * self.metrics.cell + 2.0 * TEXT_PAD - track.w).max(0.0);
        view.scroll_x = view.scroll_x.clamp(0.0, view.max_scroll_x);
    }

    /// 返回 Properties 预览竖直滚动条的轨道、滚动位置与内容高度，单位为逻辑像素。
    pub(crate) fn properties_vscrollbar(&self) -> Option<(Rect, f32, f32)> {
        let area = self.properties_ready()?;
        let line_count = self.editor.active()?.text().line_count();
        let lh = self.fonts.line_height;
        Some((
            area,
            (self.preview.properties.top_line * f64::from(lh)) as f32,
            editor_content_height(line_count, area.h, lh),
        ))
    }

    /// 返回 Properties 预览水平滚动条的轨道、滚动位置与内容宽度，单位为逻辑像素。
    pub(crate) fn properties_hscrollbar(&self) -> Option<(Rect, f32, f32)> {
        let area = self.properties_ready()?;
        let track = horizontal_track(area);
        let view = &self.preview.properties;
        Some((track, view.scroll_x, track.w + view.max_scroll_x))
    }

    /// 预览已为当前标签准备好时返回它的区域。
    fn properties_ready(&self) -> Option<Rect> {
        let area = self.properties_area()?;
        let doc = self.editor.active()?;
        (self.preview.properties.doc == Some(doc.id)).then_some(area)
    }

    pub(in crate::app) fn drag_properties_scrollbar(
        &mut self,
        horizontal: bool,
        (x, y): (f32, f32),
        grab: f32,
    ) {
        self.prepare_properties();
        let bar = if horizontal {
            self.properties_hscrollbar()
        } else {
            self.properties_vscrollbar()
        };
        let Some((track, _, content)) = bar else {
            return;
        };
        if horizontal {
            if let Some(thumb) = hscrollbar_thumb(track, 0.0, content) {
                self.preview.properties.scroll_x =
                    scroll_for_thumb(track.x, track.w, thumb.w, x, grab, content - track.w);
            }
        } else if let Some(thumb) = scrollbar_thumb(track, 0.0, content) {
            let y = scroll_for_thumb(track.y, track.h, thumb.h, y, grab, content - track.h);
            self.scroll_properties_to(f64::from(y) / f64::from(self.fonts.line_height));
        }
        self.request_redraw();
    }

    pub(in crate::app) fn scroll_properties_by(&mut self, dx: f32, dy: f32) {
        self.settle_editor_glide();
        self.prepare_properties();
        if dx.abs() > dy.abs() {
            let view = &mut self.preview.properties;
            view.scroll_x = (view.scroll_x - dx).clamp(0.0, view.max_scroll_x);
        } else if dy != 0.0 {
            let line = self.preview.properties.top_line
                - f64::from(dy) / f64::from(self.fonts.line_height);
            self.scroll_properties_to(line);
        }
    }

    /// 预览中的导航键滚动视口，不移动源代码的光标或选区。
    pub(in crate::app) fn properties_key(&mut self, key: &Key, cmd: bool) -> bool {
        let Some(area) = self.properties_area().filter(|_| self.properties_focused()) else {
            return false;
        };
        self.prepare_properties();
        let line = self.preview.properties.top_line;
        let page = f64::from((area.h / self.fonts.line_height).floor().max(2.0) - 1.0);
        let back = self.pointer.modifiers.shift_key();
        let to = match key {
            Key::Named(NamedKey::Home) => 0.0,
            Key::Named(NamedKey::End) => f64::MAX,
            Key::Named(NamedKey::ArrowUp) if cmd => 0.0,
            Key::Named(NamedKey::ArrowDown) if cmd => f64::MAX,
            Key::Named(NamedKey::ArrowUp) => line - 1.0,
            Key::Named(NamedKey::ArrowDown) => line + 1.0,
            Key::Named(NamedKey::PageUp) => line - page,
            Key::Named(NamedKey::PageDown) => line + page,
            Key::Named(NamedKey::Space) => line + if back { -page } else { page },
            Key::Character(c) if c == " " => line + if back { -page } else { page },
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                let right = *key == Key::Named(NamedKey::ArrowRight);
                let step = if cmd {
                    f32::MAX
                } else {
                    self.metrics.cell * 4.0
                };
                self.scroll_properties_by(if right { -step } else { step }, 0.0);
                self.request_redraw();
                return true;
            }
            _ => return false,
        };
        self.scroll_properties_to(to);
        self.request_redraw();
        true
    }

    fn scroll_properties_to(&mut self, line: f64) {
        let (Some(area), Some(doc)) = (self.properties_area(), self.editor.active()) else {
            return;
        };
        let max = editor_max_scroll(doc.text().line_count(), area.h, self.fonts.line_height);
        let line = line.clamp(0.0, max);
        self.preview.properties.top_line = line;
        self.settle_editor_glide();
        self.scroll_text_to(line);
        self.remember_properties_source_line();
        self.prepare_properties();
    }

    pub(in crate::app) fn focus_properties(&mut self) {
        self.prepare_properties();
        self.preview.properties.focused = true;
        self.focus = Focus::Editor;
        self.ime.preedit.clear();
    }

    /// 把键盘交回源代码。点击、切换标签等离开预览时调用。
    pub(in crate::app) fn unfocus_properties(&mut self) {
        self.preview.properties.focused = false;
        self.preview.properties.selection = None;
    }
}

impl PropertiesPreview {
    fn show(&mut self, doc: DocumentId, line: f64) {
        if self.doc != Some(doc) {
            *self = Self {
                doc: Some(doc),
                ..Self::default()
            };
        }
        self.top_line = line;
    }
}

fn horizontal_track(area: Rect) -> Rect {
    Rect::new(area.x, area.y, (area.w - SCROLLBAR_HIT).max(0.0), area.h)
}

/// 返回一行的预览文字，按解码后的字符数限制绘制量。
fn decoded_line(doc: &Document, line: usize) -> String {
    // 一个显示字符最多需要两个六字节的 Unicode 转义，额外读一个字符用来判断是否截断。
    const MAX_ENCODED_LINE_BYTES: usize = (MAX_LINE_CHARS + 1) * 12;
    let content = doc.text().line_content(line);
    let slice = content.slice();
    let prefix = if slice.len_bytes() > MAX_ENCODED_LINE_BYTES {
        slice.slice(..slice.byte_to_char(MAX_ENCODED_LINE_BYTES))
    } else {
        slice
    };
    let mut text = match prefix.as_str() {
        Some(text) => winc_preview::properties::decode(text),
        None => winc_preview::properties::decode(&prefix.to_string()),
    };
    if let Some((end, _)) = text.char_indices().nth(MAX_LINE_CHARS) {
        text.truncate(end);
        text.push('…');
    }
    text
}
