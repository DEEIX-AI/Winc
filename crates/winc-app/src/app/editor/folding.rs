//! 折叠代码的界面：折叠命令、行号栏中的箭头、已折叠行末尾的 `⋯`、折叠和展开的动画，
//! 以及滚动到某一行时先展开其所在的折叠块。

use std::ops::Range;

use winc_editor::{Document, DocumentId, FoldCommand, Folds};
use winc_i18n::t;

use super::*;
use crate::layout::hit;

/// 返回行号栏中绘制折叠箭头的区域：行号右边、文字左边的空白，宽度为 `GUTTER_PADDING`。
pub(crate) fn fold_column(gutter: Rect) -> Rect {
    Rect::new(
        gutter.right() - GUTTER_PADDING,
        gutter.y,
        GUTTER_PADDING,
        gutter.h,
    )
}

/// 折叠或展开一个块的动画。
///
/// 变化的几行从块开头一行的下方逐渐露出或收回，其下的行随之移动，箭头同时转向。绘制时按展开
/// 状态（`open`，滚动位置为 `scroll`）排版：变化的几行（`rows`，按展开状态计数的显示行）位置
/// 不变，只显示已露出的部分，其后的行收起尚未露出的高度（见 [`FoldMotion::hidden`]）。「全部
/// 折叠」这类同时改变多处的操作不做动画；文字被修改、滚动位置改变或切换标签时直接绘制为终态。
#[derive(Debug, Clone)]
pub struct FoldMotion {
    /// 动画所属的文档。
    pub doc: DocumentId,
    /// 动画开始时的文档版本号（`revision`）；版本号变化后动画不再适用。
    pub revision: u64,
    /// 展开状态下的折叠信息，绘制时据此排版。
    pub open: Folds,
    /// 变化的显示行范围，按展开状态计数（从 0 起）。
    pub rows: Range<usize>,
    /// 是否为展开；`false` 表示折叠。
    pub expanding: bool,
    /// 按 `open` 计数的滚动位置（显示行）。
    pub scroll: f32,
    /// 动画结束后的滚动位置（显示行），按文档当前的折叠状态计数。
    pub settled: f32,
    /// 动画的时钟。
    pub clock: Clock,
}

impl FoldMotion {
    /// 返回变化的几行已露出的比例：0 表示完全折叠，1 表示完全展开。
    pub fn reveal(&self) -> f32 {
        let eased = self.clock.progress();
        if self.expanding { eased } else { 1.0 - eased }
    }

    /// 返回变化的几行尚未露出的高度（逻辑像素），其后的行向上收起相应高度。
    pub fn hidden(&self, line_height: f32) -> f32 {
        (1.0 - self.reveal()) * self.rows.len() as f32 * line_height
    }

    /// 动画是否仍然适用于 `doc`。
    ///
    /// 动画期间文档被修改、滚动位置改变或切换了标签后不再适用，此时不再绘制动画。
    pub fn applies_to(&self, doc: &Document) -> bool {
        doc.id == self.doc && doc.revision == self.revision && doc.scroll_line == self.settled
    }
}

impl App {
    /// 执行折叠命令 `command`。
    ///
    /// 屏幕最上面的一行保持在原处；光标被折进块内时，移到该块的开头一行。
    pub(in crate::app) fn run_fold(&mut self, command: FoldCommand) {
        let tab_width = self.editor.active_tab_width();
        let Some(doc) = self.editor.active() else {
            return;
        };
        if !doc.can_fold() {
            if doc.shows_text() {
                self.flash(t!("editor-fold-large"));
            }
            return;
        }
        self.keep_top_line(|doc| doc.run_fold(command, tab_width));
    }

    /// 处理对行号栏中第 `line` 行（从 0 起）箭头的点击：切换该行的折叠状态。
    pub(in crate::app) fn click_fold(&mut self, line: usize) {
        let tab_width = self.editor.active_tab_width();
        self.focus = Focus::Editor;
        self.keep_top_line(|doc| doc.toggle_fold_at(line, tab_width));
    }

    /// 处理对已折叠行末尾 `⋯` 的点击：展开第 `line` 行（从 0 起）的折叠块。
    pub(in crate::app) fn click_folded(&mut self, line: usize) {
        self.focus = Focus::Editor;
        self.keep_top_line(|doc| doc.unfold_at(line));
    }

    /// 执行 `change` 修改折叠状态，并保持屏幕最上面的一行不动。
    ///
    /// 最上面的一行被折进块内时，保持的是该块的开头一行。`change` 返回 `true` 表示折叠状态
    /// 确实发生了变化，此时才会修正滚动位置并启动动画。光标原本不在屏幕上时（用滚轮滚开以后
    /// 在行号栏点箭头）不把它滚动到可见范围：否则最上面的一行变了，动画随之放弃。
    fn keep_top_line(&mut self, change: impl FnOnce(&mut Document) -> bool) {
        let clock = self.clock(FOLD_ANIMATION);
        let visible = self.visible_lines() as f32;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let before = (doc.folds().clone(), doc.scroll_line);
        let top = doc.line_of_row(doc.scroll_line.max(0.0) as usize);
        let fraction = doc.scroll_line.fract();
        let caret = doc.row_at(doc.selection.head) as f32;
        let caret_shown = (doc.scroll_line..doc.scroll_line + visible).contains(&caret);
        if !change(doc) {
            return;
        }
        doc.scroll_line = doc.row_of(top) as f32 + fraction;
        self.sticky_cache = None;
        self.clamp_scroll();
        if caret_shown {
            self.ensure_caret_visible();
        }
        self.fold_motion = clock.and_then(|clock| self.fold_motion(before, clock));
        self.reset_blink();
        self.request_redraw();
    }

    /// 根据 `before`（修改之前的折叠状态和滚动位置）与当前状态，生成折叠变化的动画。
    ///
    /// 不适合做动画时返回 `None`。同时满足以下条件才做动画：屏幕最上面的一行没有变；
    /// 变化的显示行连成一片（只有一处）；这片行位于最上面一行之下。滚动位置发生了变化
    /// （光标被折进块内、滚到底部被限制回来）时，直接变为最终状态。
    pub(in crate::app) fn fold_motion(
        &self,
        before: (Folds, f32),
        clock: Clock,
    ) -> Option<FoldMotion> {
        let doc = self.editor.active()?;
        let (folds, scroll) = before;
        let now = doc.folds();
        let (open, rows, expanding, open_scroll) =
            if let Some(rows) = Folds::changed_rows(&folds, now) {
                (folds, rows, false, scroll)
            } else {
                let rows = Folds::changed_rows(now, &folds)?;
                (now.clone(), rows, true, doc.scroll_line)
            };
        let top = |folds: &Folds, scroll: f32| folds.line_of_row(scroll.max(0.0) as usize);
        let same_top = top(&open, open_scroll) == top(now, doc.scroll_line)
            && open_scroll.fract() == doc.scroll_line.fract();
        (same_top && rows.start > open_scroll.floor() as usize).then_some(FoldMotion {
            doc: doc.id,
            revision: doc.revision,
            open,
            rows,
            expanding,
            scroll: open_scroll,
            settled: doc.scroll_line,
            clock,
        })
    }

    /// 滚动到第 `line` 行（从 0 起），使该行位于可见区域从顶部起 `at` 比例处。
    ///
    /// `at` 为 0 表示顶部，0.5 表示中间。
    ///
    /// 该行在折叠块内时先展开。`only_if_off` 为 `true` 时，行已经可见则不滚动。
    pub(in crate::app) fn scroll_to_line(&mut self, line: usize, at: f32, only_if_off: bool) {
        let visible = self.visible_lines() as f32;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        doc.reveal(
            doc.text()
                .line_start(line.min(doc.text().line_count().saturating_sub(1))),
        );
        let row = doc.row_of(line) as f32;
        if only_if_off && row >= doc.scroll_line && row < doc.scroll_line + visible - 1.0 {
            return;
        }
        let max = self.max_scroll_line();
        if let Some(doc) = self.editor.active_mut() {
            let before = doc.scroll_line;
            doc.scroll_line = (row - visible * at).clamp(0.0, max);
            self.glide_scroll(before);
        }
    }

    /// 返回折叠箭头区域中窗口坐标 (`x`, `y`)（逻辑像素）处的行号（从 0 起）。
    ///
    /// 只有该行是块的开头或已折叠时才返回，其他位置返回 `None`。箭头区域之外不处理，
    /// 点击行号本身仍是选中整行。
    pub(in crate::app) fn fold_at(&mut self, x: f32, y: f32) -> Option<usize> {
        let gutter = self.layout.gutter;
        if !gutter.contains(x, y) || y < self.layout.text.y || x < fold_column(gutter).x {
            return None;
        }
        let tab_width = self.editor.active_tab_width();
        let line_height = self.fonts.line_height;
        let doc = self.editor.active_mut()?;
        let offset = (y - self.layout.text.y) / line_height;
        let row = (offset + doc.scroll_line).floor() as usize;
        if row >= doc.row_count() {
            return None;
        }
        // 自动换行时箭头只画在一行的第一个显示行旁边。第一个显示行已滚出编辑区顶部时，
        // 箭头画在可见的最上面一个显示行旁边，行号也按此规则绘制；点击该处同样作用于这一行。
        let (line, part) = doc.line_and_part(row);
        let pinned = offset < 1.0 && (doc.row_of(line) as f32) < doc.scroll_line;
        if part > 0 && !pinned {
            return None;
        }
        (doc.folds().is_collapsed(line) || doc.starts_block(line, tab_width)).then_some(line)
    }

    /// 返回窗口坐标 (`x`, `y`)（逻辑像素）处已折叠行末尾 `⋯` 对应的行号（从 0 起）。
    ///
    /// 该处没有 `⋯` 时返回 `None`。位置取自上一次绘制时记录的 `folded_hits`。
    pub(in crate::app) fn folded_at(&self, x: f32, y: f32) -> Option<usize> {
        hit(&self.folded_hits, x, y)
    }
}
