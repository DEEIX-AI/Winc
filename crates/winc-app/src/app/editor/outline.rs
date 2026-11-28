//! 编辑区右上角的大纲快速查看：指示条、悬停展开的悬浮卡片与图钉固定。
//!
//! 交互与 Markdown 预览左侧的标题指示条一致：平时在编辑区右上角显示紧凑的进度指示条，
//! 鼠标停留满 `TOC_HOVER_DELAY` 后展开悬浮大纲卡片，点击一项跳转到对应符号；点击卡片右上角的
//! 图钉可固定展开，再点一次取消。

use std::time::Instant;

use super::*;
use crate::app::animation::{self, Clock};
use crate::app::preview::markdown::TocList;

/// 编辑区右上角大纲指示条与悬浮卡片上可以指向的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorOutlineTarget {
    /// 指示条本身或卡片内边距：鼠标停在上面时展开大纲列表。
    Bars,
    /// 大纲列表里的一项，值为 [`OutlineState::items`] 的下标：点击后跳转到该符号。
    Item(usize),
    /// 列表右上角的图钉：钉住后列表一直展开，再点一次取消。
    Pin,
}

/// 编辑区右上角大纲悬浮卡片的动画与滚动状态。
#[derive(Default)]
pub(crate) struct EditorOutlineState {
    /// 列表展开时淡入的时钟；不在淡入中时为 `None`。
    pub fade: Option<Clock>,
    /// 鼠标进入指示条的时刻，满 `TOC_HOVER_DELAY` 后列表展开；
    /// 不在指示条和列表上时为 `None`。
    pub rest: Option<Instant>,
    /// 本次停留是否已展开列表。
    pub shown: bool,
    /// 鼠标进入（`true`）或离开（`false`）指示条时指示条加深、变淡的时钟；不在过渡中时为 `None`。
    pub glow: Option<(bool, Clock)>,
    /// 列表收起过渡的时钟；不在收起中时为 `None`。
    pub close: Option<Clock>,
    /// 列表钉住（`true`）或取消钉住（`false`）的过渡时钟；不在过渡中时为 `None`。
    pub pin: Option<(bool, Clock)>,
    /// 展开的列表列出的一段，供滚轮滚动与滚动条拖动。
    pub list: TocList,
}

impl App {
    /// 编辑区右上角的大纲指示条是否启用：设置未关闭、当前标签显示文本区且文档可提取符号。
    pub(crate) fn editor_outline_active(&self) -> bool {
        self.config.outline != Some(false)
            && self.active_shows_text()
            && self
                .editor
                .active()
                .is_some_and(|doc| doc.edits_text() && !doc.is_large())
    }

    /// 处理编辑区右上角大纲卡片中的点击：图钉切换固定状态，符号项跳转到对应位置。
    pub(in crate::app) fn press_editor_outline(&mut self, target: EditorOutlineTarget) {
        match target {
            EditorOutlineTarget::Pin => {
                self.update_state(move |state| {
                    state.editor_outline_pinned = !state.editor_outline_pinned;
                });
                let pinned = self.state.editor_outline_pinned;
                let clock = self.clock(animation::TOC_PIN);
                self.caches.outline_toc.pin = clock.map(|clock| (pinned, clock));
                self.request_redraw();
            }
            EditorOutlineTarget::Item(i) => self.jump_to_outline(i),
            EditorOutlineTarget::Bars => {}
        }
    }

    /// 拖动编辑区大纲悬浮卡片的滚动条：鼠标纵坐标为 `y`，按下处距滑块上边 `grab`（逻辑像素）。
    pub(crate) fn drag_editor_outline_scrollbar(&mut self, y: f32, grab: f32) {
        let list = &mut self.caches.outline_toc.list;
        let Some((_, content)) = list.scrollbar() else {
            return;
        };
        let track = list.track;
        if let Some(thumb) = crate::layout::scrollbar_thumb(track, 0.0, content) {
            let max = content - track.h;
            let to = crate::layout::scroll_for_thumb(track.y, track.h, thumb.h, y, grab, max);
            let first = (to / list.row).round() as isize;
            list.shift += first - list.first as isize;
            self.request_redraw();
        }
    }

    /// 按滚轮位移 `dy` 滚动编辑区大纲悬浮卡片，返回是否已处理。
    pub(crate) fn scroll_editor_outline(&mut self, (x, y): (f32, f32), dy: f32) -> bool {
        let list = &mut self.caches.outline_toc.list;
        if list.rows >= list.total || list.row <= 0.0 || !list.card.contains(x, y) {
            return false;
        }
        list.wheel -= dy;
        let steps = (list.wheel / list.row).trunc();
        list.wheel -= steps * list.row;
        list.shift += steps as isize;
        self.request_redraw();
        true
    }
}
