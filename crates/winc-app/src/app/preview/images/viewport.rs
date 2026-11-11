//! 图片预览的几何与交互状态；缩放不改变解码后的图片。

use std::sync::{Arc, Weak};

use winc_editor::{DocumentId, PreviewMode};
use winc_preview::image::{Picture, Source};
use winc_render::Rect;

use crate::app::{App, Drag, Focus};

/// 相对初始适应尺寸的缩放范围，限制极端输入造成的坐标增长。
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.1..=64.0;

/// 当前图片的视图状态。只弱引用图片，隐藏标签仍可释放解码缓存。
pub(crate) struct PictureView {
    pub(crate) doc: Option<DocumentId>,
    picture: Weak<Picture>,
    pub(crate) area: Rect,
    size: (f32, f32),
    pub(crate) zoom: f32,
    pub(crate) pan: (f32, f32),
}

impl Default for PictureView {
    fn default() -> Self {
        Self {
            doc: None,
            picture: Weak::new(),
            area: Rect::default(),
            size: (0.0, 0.0),
            zoom: 1.0,
            pan: (0.0, 0.0),
        }
    }
}

impl PictureView {
    /// 同步绘制区域和初始适应尺寸；换文档或图片后恢复初始视图。
    fn sync(&mut self, id: DocumentId, picture: &Arc<Picture>, area: Rect) -> bool {
        let changed = self.doc != Some(id) || self.picture.as_ptr() != Arc::as_ptr(picture);
        if changed {
            *self = Self {
                doc: Some(id),
                picture: Arc::downgrade(picture),
                ..Self::default()
            };
        }
        self.area = area;
        let room = area.inset(24.0, 24.0);
        let (w, h) = (picture.width as f32, picture.height as f32);
        // 小 SVG 的长边至少为 256 逻辑像素；位图默认不超过原始大小。
        let natural = match picture.source {
            Source::Vector(_) => (256.0 / w.max(h)).max(1.0),
            Source::Raster(_) => 1.0,
        };
        let fit = (room.w / w).min(room.h / h).min(natural).max(0.0);
        self.size = (w * fit, h * fit);
        changed
    }

    /// 返回缩放和平移后的图片矩形（逻辑像素）。
    pub(crate) fn rect(&self) -> Rect {
        let (w, h) = (self.size.0 * self.zoom, self.size.1 * self.zoom);
        Rect::new(
            self.area.x + (self.area.w - w) / 2.0 + self.pan.0,
            self.area.y + (self.area.h - h) / 2.0 + self.pan.1,
            w,
            h,
        )
    }

    /// 以指针位置为锚点缩放；使用限制后的实际倍率计算偏移。
    pub(crate) fn zoom_at(&mut self, steps: f32, mouse: (f32, f32)) {
        if !steps.is_finite() || steps == 0.0 {
            return;
        }
        let to = (self.zoom * 1.2_f32.powf(steps.clamp(-64.0, 64.0)))
            .clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        let ratio = to / self.zoom;
        let anchor = (
            mouse.0 - self.area.x - self.area.w / 2.0,
            mouse.1 - self.area.y - self.area.h / 2.0,
        );
        self.pan = (
            anchor.0 + (self.pan.0 - anchor.0) * ratio,
            anchor.1 + (self.pan.1 - anchor.1) * ratio,
        );
        self.zoom = to;
    }
}

impl App {
    /// 新图片开始显示时取消旧拖动；旧拖动不能重新应用到已复位的视图。
    pub(crate) fn sync_picture_view(&mut self, id: DocumentId, picture: &Arc<Picture>, area: Rect) {
        if self.picture_view.sync(id, picture, area) {
            self.cancel_picture_drag();
        }
    }

    /// 离开图片预览时清除几何和拖动状态。
    pub(crate) fn reset_picture_view(&mut self) {
        self.picture_view = PictureView::default();
        self.cancel_picture_drag();
    }

    /// 取消图片拖动，不影响其他控件的拖动状态。
    pub(in crate::app) fn cancel_picture_drag(&mut self) {
        if matches!(self.drag, Drag::Picture { .. }) {
            self.drag = Drag::None;
            self.update_cursor_icon();
        }
    }

    /// 当前文档仍在显示这张图片时，返回鼠标是否位于图片预览区域。
    pub(crate) fn picture_at(&self, x: f32, y: f32) -> bool {
        let view = &self.picture_view;
        let Some(doc) = self.editor.active().filter(|d| view.doc == Some(d.id)) else {
            return false;
        };
        let (picture, area) = if let Some(picture) = doc.picture() {
            (Some(picture), self.layout.editor)
        } else if doc.preview_mode() == PreviewMode::Split && self.split.doc == Some(doc.id) {
            (self.split.picture.as_ref(), self.layout.split)
        } else {
            (None, Rect::default())
        };
        view.area == area
            && area.contains(x, y)
            && picture.is_some_and(|p| Arc::as_ptr(p) == view.picture.as_ptr())
    }

    /// 左键拖动图片；双击恢复适应窗口。
    pub(in crate::app) fn click_picture(&mut self, x: f32, y: f32) {
        self.focus = Focus::Editor;
        if self.click_count(x, y) == 2 {
            self.picture_view.zoom = 1.0;
            self.picture_view.pan = (0.0, 0.0);
            self.drag = Drag::None;
        } else if let Some(doc) = self.picture_view.doc {
            self.drag = Drag::Picture {
                doc,
                start: (x, y),
                pan: self.picture_view.pan,
            };
        }
        self.update_cursor_icon();
        self.request_redraw();
    }
}
