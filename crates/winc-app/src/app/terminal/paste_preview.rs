//! 从程序实际绘出的 [Image #N] 关联刚粘贴的剪贴板图像。

use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;
use std::time::{Duration, Instant};

use winc_preview::image::{self, Pixels};
use winc_pty::{protocol::Line, width::cells};
use winc_render::Rect;

use super::{App, Focus, TermTab};
use crate::app::editor::clipboard::ClipImage;

const MAX_IMAGES: usize = 16;
const PASTE_TIMEOUT: Duration = Duration::from_secs(5);

type Marks = BTreeMap<u32, Vec<(u64, Range<usize>)>>;

#[derive(Default)]
pub(in crate::app) struct PastePreview {
    pending: VecDeque<(Instant, Pixels)>,
    images: BTreeMap<u32, Pixels>,
    marks: Marks,
    deletion: Option<(Instant, u32)>,
}

impl PastePreview {
    pub(in crate::app) fn queue(&mut self, pixels: Pixels, rows: &[Line], first: u64) {
        self.observe(rows, first);
        self.discard_missing();
        if self.pending.is_empty() {
            self.marks = markers(rows, first);
        }
        if self.pending.len() == MAX_IMAGES {
            self.pending.pop_front();
        }
        self.pending.push_back((Instant::now(), pixels));
    }

    pub(in crate::app) fn observe(&mut self, rows: &[Line], first: u64) {
        if self.pending.is_empty() && self.images.is_empty() {
            return;
        }
        let marks = markers(rows, first);
        self.pending.retain(|(at, _)| at.elapsed() < PASTE_TIMEOUT);
        if self.reconcile_deletion(&marks) {
            self.marks = marks;
            return;
        }
        // 编号由程序决定。已有标记移动时出现次数不变；新出现的编号才消耗待匹配的图片。
        // 重绘可能暂时不显示已有标记，恢复时沿用其图片。
        for (&number, occurrences) in &marks {
            let old = self.marks.get(&number).map_or(0, Vec::len);
            for _ in old..occurrences.len() {
                let Some((_, pixels)) = self.pending.pop_front() else {
                    if old > 0 {
                        self.images.remove(&number);
                    }
                    break;
                };
                self.images.insert(number, pixels);
                if self.images.len() > MAX_IMAGES {
                    self.images.pop_first();
                }
            }
        }
        self.marks = marks;
    }

    /// 开始编辑或粘贴时回收已不可见的附件，避免草稿复用编号后匹配到旧图片。
    pub(super) fn discard_missing(&mut self) {
        self.images
            .retain(|number, _| self.marks.contains_key(number));
    }

    fn note_delete(&mut self, line: u64, col: usize, backwards: bool) {
        let number = self.marks.iter().find_map(|(&number, occurrences)| {
            let (row, columns) = occurrences.last()?;
            let cell = if backwards { col.checked_sub(1)? } else { col };
            (*row == line && columns.contains(&cell)).then_some(number)
        });
        self.deletion = number.map(|number| (Instant::now(), number));
    }

    /// 用删除键所在的标记区分保留原编号和重排编号的程序。
    fn reconcile_deletion(&mut self, marks: &Marks) -> bool {
        let Some((at, removed)) = self.deletion else {
            return false;
        };
        if at.elapsed() >= PASTE_TIMEOUT {
            self.deletion = None;
            return false;
        }
        let count = |marks: &Marks| marks.values().map(Vec::len).sum::<usize>();
        if count(&self.marks).checked_sub(1) != Some(count(marks)) {
            return false;
        }
        self.deletion = None;

        // 旧输出中同编号的标记可能仍在屏幕上，只移除或重排每个编号的最后一次出现。
        let numbers = |marks: &Marks| {
            marks
                .iter()
                .flat_map(|(&n, hits)| std::iter::repeat_n(n, hits.len()))
                .collect::<Vec<_>>()
        };
        let mut expected = numbers(&self.marks);
        let actual = numbers(marks);
        let Some(index) = expected.iter().rposition(|&n| n == removed) else {
            return false;
        };
        expected.remove(index);
        if expected == actual {
            self.images.remove(&removed);
            return true;
        }
        for (&number, _) in self.images.range(removed.saturating_add(1)..) {
            if let Some(number) = expected.iter_mut().rfind(|n| **n == number) {
                *number -= 1;
            }
        }
        expected.sort_unstable();
        if expected != actual {
            return false;
        }
        self.images.remove(&removed);
        self.images = std::mem::take(&mut self.images)
            .into_iter()
            .map(|(n, pixels)| (if n > removed { n - 1 } else { n }, pixels))
            .collect();
        true
    }

    fn at(&self, line: u64, col: usize) -> Option<(&Pixels, &Range<usize>)> {
        self.images.iter().find_map(|(number, pixels)| {
            let (row, range) = self.marks.get(number)?.last()?;
            (*row == line && range.contains(&col)).then_some((pixels, range))
        })
    }
}

fn markers(rows: &[Line], first: u64) -> Marks {
    let mut markers = BTreeMap::<_, Vec<_>>::new();
    for (row, line) in rows.iter().enumerate() {
        let text = &line.text;
        for (start, prefix) in text.match_indices("[Image #") {
            let tail = &text[start + prefix.len()..];
            let Some((digits, _)) = tail.split_once(']') else {
                continue;
            };
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(number) = digits.parse::<u32>() {
                let column = cells(&text[..start]).map(|(_, width)| width).sum::<usize>();
                let end = column + prefix.len() + digits.len() + 1;
                markers
                    .entry(number)
                    .or_default()
                    .push((first + row as u64, column..end));
            }
        }
    }
    markers
}

impl App {
    /// 观察终端发送的图片粘贴和删除按键。
    pub(super) fn track_terminal_image_input(&mut self, bytes: &[u8]) {
        if matches!(bytes, b"\x08" | b"\x7f" | b"\x1b[3~") {
            if let Some(TermTab {
                paste_preview,
                cursor,
                history,
                ..
            }) = self.terminal.tab_mut()
            {
                paste_preview.note_delete(
                    history.1 + u64::from(cursor.row),
                    usize::from(cursor.col),
                    bytes != b"\x1b[3~",
                );
            }
            return;
        }
        if !matches!(
            bytes,
            b"\x16"
                | b"\x1bv"
                | b"\x1b\x16"
                | b"\x1b[118;3u"
                | b"\x1b[118;5u"
                | b"\x1b[118;7u"
                | b"\x1b[200~\x1b[201~"
        ) || self.terminal.client.is_none()
            || self.terminal_at_prompt()
        {
            return;
        }
        let Some(tab) = self.terminal.tab().filter(|tab| {
            tab.ended.is_none()
                && tab.modes.has(super::Modes::BRACKETED_PASTE)
                && !winc_backend::is_remote(&super::bytes_path(&tab.cwd))
        }) else {
            return;
        };
        let id = tab.id;
        if self.clipboard_text().is_some_and(|text| !text.is_empty()) {
            return;
        }
        if let Some(pixels) = self.clipboard.image().and_then(paste_thumbnail)
            && let Some(tab) = self.terminal.tab_mut().filter(|tab| tab.id == id)
        {
            tab.paste_preview.queue(pixels, &tab.rows, tab.history.1);
        }
    }

    pub(crate) fn hovered_terminal_image(&self) -> Option<(&Pixels, Rect)> {
        self.cursor_inside().then_some(())?;
        self.terminal_image_hover(self.pointer.mouse)
    }

    pub(in crate::app) fn terminal_image_hover(
        &self,
        (x, y): (f32, f32),
    ) -> Option<(&Pixels, Rect)> {
        if self.focus != Focus::Terminal
            || self.pointer.drag != crate::app::Drag::None
            || !self.terminal_group_focused_at(x, y)
        {
            return None;
        }
        let tab = self.terminal.tab()?;
        if tab.ended.is_some() || tab.paste_preview.images.is_empty() {
            return None;
        }
        let view = self.terminal_view();
        let grid = self.terminal_grid();
        if !view.contains(x, y) || x < grid.x || x >= grid.right() || self.terminal_on_sticky(y) {
            return None;
        }
        let point = self.terminal_point(x, y, false)?;
        let (pixels, columns) = tab.paste_preview.at(point.line, point.col)?;
        let (_, origin) = self.terminal_origin();
        let lh = self.fonts.terminal_line_height;
        let top = origin + ((y - origin) / lh).floor() * lh;
        let cell = self.metrics.terminal_cell;
        Some((
            pixels,
            Rect::new(
                grid.x + columns.start as f32 * cell,
                top,
                columns.len() as f32 * cell,
                lh,
            )
            .intersect(view),
        ))
    }
}

/// 限制留在终端会话中的图片尺寸，并保留原比例。
fn paste_thumbnail(image: ClipImage) -> Option<Pixels> {
    const MAX_EDGE: u32 = 256;
    let pixels = match image {
        ClipImage::Png(png) => image::decode_bytes(&png).ok()?,
        ClipImage::Rgba {
            width,
            height,
            bytes,
        } => Pixels {
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            rgba: bytes,
        },
    };
    let edge = pixels.width.max(pixels.height);
    if pixels.width == 0 || pixels.height == 0 {
        return None;
    }
    if edge <= MAX_EDGE {
        return Some(pixels);
    }
    let scaled = |size| ((u64::from(size) * u64::from(MAX_EDGE) / u64::from(edge)) as u32).max(1);
    Some(image::resize(
        &pixels,
        scaled(pixels.width),
        scaled(pixels.height),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> [Line; 1] {
        [Line {
            text: text.into(),
            ..Line::default()
        }]
    }

    fn three_images() -> PastePreview {
        let mut preview = PastePreview::default();
        for value in [1, 2, 3] {
            preview.queue(
                Pixels {
                    width: 1,
                    height: 1,
                    rgba: vec![value, 0, 0, 255],
                },
                &line(""),
                10,
            );
        }
        preview.observe(&line("[Image #1] [Image #2] [Image #3]"), 10);
        preview
    }

    #[test]
    fn removing_an_image_keeps_previews_with_unchanged_numbers() {
        let mut preview = three_images();
        preview.observe(&line("[Image #1] [Image #3]"), 10);
        assert_eq!(preview.at(10, 0).unwrap().0.rgba[0], 1);
        assert_eq!(preview.at(10, 11).unwrap().0.rgba[0], 3);
        assert!(preview.at(10, 22).is_none());
    }

    #[test]
    fn a_new_edit_does_not_reuse_a_preview_from_a_missing_marker() {
        let mut preview = three_images();
        preview.observe(&line(""), 10);
        preview.discard_missing();
        preview.observe(&line("[Image #1]"), 10);
        assert!(preview.at(10, 0).is_none());
    }

    #[test]
    fn removing_an_image_rebinds_renumbered_previews_to_the_same_pixels() {
        for (col, backwards, expected) in
            [(10, true, [2, 3]), (11, false, [1, 3]), (32, true, [1, 2])]
        {
            let mut preview = three_images();
            preview.note_delete(10, col, backwards);
            preview.observe(&line("[Image #1] [Image #2]"), 10);
            assert_eq!(preview.at(10, 0).unwrap().0.rgba[0], expected[0]);
            assert_eq!(preview.at(10, 11).unwrap().0.rgba[0], expected[1]);
            assert!(preview.at(10, 22).is_none());
        }
    }

    #[test]
    fn paste_thumbnail_preserves_aspect_ratio_and_limits_retained_pixels() {
        let rgba = [32, 64, 96, 255].repeat(800 * 400);
        let png = winc_editor::paste::encode_png(800, 400, &rgba).unwrap();
        let thumbnail = paste_thumbnail(ClipImage::Png(png)).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (256, 128));
        assert_eq!(thumbnail.rgba, [32, 64, 96, 255].repeat(256 * 128));
    }
}
