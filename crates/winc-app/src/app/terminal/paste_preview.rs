//! 从程序实际绘出的 [Image #N] 关联刚粘贴的剪贴板图像。

use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;
use std::time::{Duration, Instant};

use winc_preview::image::{self, Pixels};
use winc_pty::{protocol::Line, width::cells};
use winc_render::Rect;

use super::{App, Focus};
use crate::app::editor::clipboard::ClipImage;

const MAX_IMAGES: usize = 16;
const PASTE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(in crate::app) struct PastePreview {
    pending: VecDeque<(Instant, Pixels)>,
    images: BTreeMap<u32, Pixels>,
    marks: BTreeMap<u32, Vec<(u64, Range<usize>)>>,
}

impl PastePreview {
    pub(in crate::app) fn queue(&mut self, pixels: Pixels, rows: &[Line], first: u64) {
        self.observe(rows, first);
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
        // 程序删掉附件后可能重排其余编号，剩下的编号无法再可靠地对应到旧图片。
        if self.images.keys().any(|number| !marks.contains_key(number)) {
            self.images.clear();
        }
        // 编号由程序决定。已有标记移动时出现次数不变；新出现的编号才消耗待匹配的图片。
        for (&number, occurrences) in &marks {
            let old = self.marks.get(&number).map_or(0, Vec::len);
            for _ in old..occurrences.len() {
                let Some((_, pixels)) = self.pending.pop_front() else {
                    self.images.remove(&number);
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

    fn at(&self, line: u64, col: usize) -> Option<(&Pixels, &Range<usize>)> {
        self.images.iter().find_map(|(number, pixels)| {
            let (row, range) = self.marks.get(number)?.last()?;
            (*row == line && range.contains(&col)).then_some((pixels, range))
        })
    }
}

fn markers(rows: &[Line], first: u64) -> BTreeMap<u32, Vec<(u64, Range<usize>)>> {
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
    /// 观察终端已经选择发送的图片粘贴键或空括号粘贴；转发的字节由原来的输入路径处理。
    pub(super) fn capture_terminal_image_paste(&mut self, bytes: &[u8]) {
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

    #[test]
    fn paste_thumbnail_preserves_aspect_ratio_and_limits_retained_pixels() {
        let rgba = [32, 64, 96, 255].repeat(800 * 400);
        let png = winc_editor::paste::encode_png(800, 400, &rgba).unwrap();
        let thumbnail = paste_thumbnail(ClipImage::Png(png)).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (256, 128));
        assert_eq!(thumbnail.rgba, [32, 64, 96, 255].repeat(256 * 128));
    }
}
