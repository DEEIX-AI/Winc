//! 松手后的放下：判断放置位置是否可用，按放置位置算出原文的改动，作为一次编辑应用到文档，
//! 并安排放下后的过渡。
//!
//! 改动经文档事务应用，不经过键入的路径：撤销一次恢复原文和原来的全部选区，重做恢复改写后
//! 的选区；文档只读时不改动原文，也不播放放下的过渡。

use std::time::Instant;

use winc_core::{Change, EditKind, Lines, Selection, Transaction};
use winc_preview::markdown::{Block, Markdown};

use super::rewrite::{TextMove, item_order, moved_block, moved_list_text};
use super::{
    BlockDrag, Boundary, DropCard, Element, ItemMotion, ReleasedShift, boundary_effective,
};
use crate::app::App;
use crate::app::preview::markdown::laid::Laid;

/// 一次放下的改动和放下之后的过渡。
pub(super) struct Landing {
    /// 原文的改动。
    pub(super) moved: TextMove,
    /// 新解析到达前留在松手位置的浮动卡片。
    card: DropCard,
    /// 顶层块的让位偏移；拖动列表项时为 `None`。
    release: Option<ReleasedShift>,
    /// 列表项的逐项滑动；拖动顶层块时为 `None`。
    items: Option<ItemStart>,
}

/// 列表项放下后逐项滑动的起点。
struct ItemStart {
    /// 列表所在的顶层块序号。
    block: usize,
    /// 松手时各项按旧顺序的上边缘（内容坐标，含让位偏移）。
    tops: Vec<f32>,
    /// 新顺序第 i 项在旧顺序中的序号。
    order: Vec<usize>,
}

impl App {
    /// 计算在 `drag` 当前的放置位置放下时的改动和过渡；文档、解析结果已变化，或放置位置
    /// 不改变顺序时返回 `None`。
    pub(super) fn plan_drop(&self, drag: &BlockDrag) -> Option<Landing> {
        let md = &self.preview.markdown;
        let doc = self.editor.active()?;
        if doc.id.0 != drag.doc || doc.revision != drag.revision {
            return None;
        }
        let (laid, parsed) = (md.layout.as_ref()?, md.parsed.as_ref()?);
        // 解析结果已更新而排版尚未更新时，旧范围不可用于改写新文字：范围可能落在 UTF-8 字符中间。
        if laid.source != md.parse_serial
            || !boundary_effective(self, parsed, drag.element, drag.target)
        {
            return None;
        }
        let area = md.area;
        let card_top = drag.card_top(area, self.mouse().1);
        let visual_top = card_top - (area.y + laid.pad - crate::layout::drawn_scroll(md.scroll()));
        let (moved, items) = self.dropped_text(drag, visual_top)?;
        let release = match (drag.element, drag.target) {
            (Element::Block(block), Boundary::Block(target)) => {
                let mut offsets = drag.shift.current.clone();
                offsets.resize(laid.blocks.len(), 0.0);
                offsets[block] = visual_top - laid.tops.get(block)?;
                let landed = if target > block { target - 1 } else { target };
                Some(ReleasedShift {
                    offsets,
                    dragged: Some((block, landed)),
                    revision: 0,
                })
            }
            _ => None,
        };
        let card = DropCard {
            block: drag.block,
            element: drag.element,
            left: drag.left,
            width: drag.width,
            height: drag.height,
            offset: drag.offset,
            top: card_top,
            shift: drag.shift.current.clone(),
            revision: 0,
        };
        Some(Landing {
            moved,
            card,
            release,
            items,
        })
    }

    /// 返回放下 `drag` 时原文的改动；拖动列表项时另外返回逐项滑动的起点。`visual_top` 是
    /// 浮动卡片松手时的上边缘（内容坐标）。
    fn dropped_text(
        &self,
        drag: &BlockDrag,
        visual_top: f32,
    ) -> Option<(TextMove, Option<ItemStart>)> {
        let md = &self.preview.markdown;
        let doc = self.editor.active()?;
        let (laid, rope) = (md.layout.as_ref()?, doc.text().rope()?);
        match (drag.element, drag.target) {
            (Element::Block(block), Boundary::Block(target)) => {
                let eol = doc.line_ending.as_str();
                Some((moved_block(rope, &laid.ranges, block, target, eol)?, None))
            }
            (Element::Item { block, item }, Boundary::Item(slot)) => {
                let range = laid.ranges.get(block)?;
                let len = rope.len_bytes();
                let start = range.start.min(len);
                let text = rope
                    .byte_slice(start..range.end.min(len).max(start))
                    .to_string();
                let list = md.drag.list_items(self, block)?;
                let moved = moved_list_text(&text, start, &list, item, slot)?;
                let items = md.drag.item_shapes(laid, block, &list).map(|shapes| {
                    let mut tops: Vec<f32> = (shapes.iter().enumerate())
                        .map(|(i, shape)| shape.top + drag.shift.at(i))
                        .collect();
                    if let Some(top) = tops.get_mut(item) {
                        *top = visual_top;
                    }
                    let order = item_order(item, slot, list.ranges.len());
                    ItemStart { block, tops, order }
                });
                Some((moved, items))
            }
            _ => None,
        }
    }

    /// 把 `moved` 作为一次编辑应用到当前文档，各选区随内容映射到新位置；返回是否已应用。
    ///
    /// 文档只读时在状态栏说明原因，不改动原文。
    pub(super) fn commit_text_move(&mut self, moved: &TextMove) -> bool {
        let Some(doc) = self.editor.active() else {
            return false;
        };
        if let Some(reason) = doc.readonly_reason() {
            self.flash(reason);
            return false;
        }
        let Some(rope) = doc.text().rope() else {
            return false;
        };
        let lines = rope.line_count();
        let char_at = |byte: usize| rope.byte_to_char(byte.min(rope.len_bytes()));
        // 各处改动按起点从后往前排列：依次应用时，每处的位置仍是改动前的字符偏移。
        let changes: Vec<Change> = (moved.edits.iter())
            .map(|edit| {
                let (start, end) = (char_at(edit.range.start), char_at(edit.range.end));
                Change {
                    start,
                    removed: rope.slice(start..end).to_string(),
                    inserted: edit.text.clone(),
                }
            })
            .collect();
        let mut transaction =
            Transaction::new(changes, doc.selection, doc.selection, EditKind::Other);
        // 选区的新位置按改动后的文字换算成字符偏移：在 rope 的副本上应用改动。
        let mut after = rope.clone();
        transaction.apply(&mut after);
        let (all, primary) = doc.selections();
        let mapped: Vec<Selection> = (all.iter())
            .map(|selection| {
                let at = |pos: usize| {
                    let byte = moved.map(rope.char_to_byte(pos.min(rope.len_chars())));
                    after.byte_to_char(byte.min(after.len_bytes()))
                };
                Selection::new(at(selection.anchor), at(selection.head))
            })
            .collect();
        let (merged, primary) = winc_core::selection::merge(mapped, primary);
        let Some(&after_primary) = merged.get(primary) else {
            return false;
        };
        transaction.extra_before = doc.extra_selections().to_vec();
        transaction.after = after_primary;
        transaction.extra_after = (merged.iter().enumerate())
            .filter(|&(i, _)| i != primary)
            .map(|(_, &selection)| selection)
            .collect();
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        doc.apply(transaction);
        doc.goal_cell = None;
        if doc.text().line_count() != lines {
            self.relayout();
        }
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
        true
    }

    /// 原文改写之后安排放下的过渡：浮动卡片留在松手位置，等新解析到达后各块或各项滑到
    /// 新位置。
    pub(super) fn land_drop(&mut self, drag: &BlockDrag, landing: Landing) {
        // 拖动改写的是文档，不是用户刚移动的光标：同步记录光标位置，避免下一次
        // `follow_live_caret` 把版本变化当成光标移动，把视口拉回原来阅读的光标处。
        if let Some(source) = self.live_caret_source()
            && let Some(doc) = self.editor.active()
        {
            self.preview.markdown.followed = Some((doc.id, source, doc.revision));
        }
        let revision = self.editor.active().map_or(0, |doc| doc.revision);
        let md = &mut self.preview.markdown;
        md.drag.drop_card = Some(DropCard {
            revision,
            ..landing.card
        });
        md.drag.release = landing.release.map(|release| ReleasedShift {
            revision,
            ..release
        });
        if let Some(items) = landing.items {
            md.drag.item_motion = Some(ItemMotion {
                block: items.block,
                revision,
                old_tops: items.tops,
                order: items.order,
                clock: None,
                created: Instant::now(),
            });
        }
        md.goal = None;
        md.line_end = None;
        md.selection = None;
        // 顶层块放下后由 `Before::motion` 滑到新位置，列表项使用单独的逐项滑动；记下版本号，
        // 之后输入文字引起的重排不按拖动处理。
        let slide = matches!(drag.element, Element::Block(_));
        md.drag.slide = slide.then_some(revision);
    }
}

/// 第 `block` 块是否可以作为放置位置：有可拖动的画面，在原文中有范围，且不是脚注。
///
/// 看不见的块（收起的一节里的块、脚注前生成的分隔线）没有画面；脚注在预览中排在文末，
/// 原文位置却可能在正文之间。
pub(super) fn visible_block(laid: &Laid, parsed: &Markdown, len: usize, block: usize) -> bool {
    let (Some(entry), Some(range)) = (laid.blocks.get(block), laid.ranges.get(block)) else {
        return false;
    };
    entry.block.height > 0.0
        && !range.is_empty()
        && range.end <= len
        && !matches!(parsed.blocks.get(block), Some(Block::Footnote(..)))
}

/// 第 `block` 块是否为收起的标题：下面一节中的块均未排版（高度为 0）。
pub(super) fn folded_heading(laid: &Laid, block: usize) -> bool {
    let Some(Some(section)) = laid.sections.get(block) else {
        return false;
    };
    (laid.blocks.get(block + 1..section.end))
        .is_some_and(|hidden| !hidden.is_empty() && hidden.iter().all(|e| e.block.height == 0.0))
}

/// 插到第 `target` 块之前（等于块数时插到文末）的位置是否落在收起的一节中。
///
/// 插到一节之后的下一个标题之前，放下的块属于这一节；最后一节之后只有脚注时，插到文末同样
/// 属于最后一节。
pub(super) fn inside_fold(laid: &Laid, parsed: &Markdown, len: usize, target: usize) -> bool {
    let count = laid.blocks.len();
    (laid.sections.iter().enumerate().take(target)).any(|(heading, section)| {
        section.as_ref().is_some_and(|section| {
            let last = target == count
                && (section.end..count).all(|i| !visible_block(laid, parsed, len, i));
            (target <= section.end || last) && folded_heading(laid, heading)
        })
    })
}

/// 当前文档是否以 front matter 开头：第一块从文首开始，原文以 `---` 开头，显示为表格或
/// 代码块。front matter 必须位于文首，不可拖动，其前面也不可放置。
pub(super) fn front_matter(app: &App) -> bool {
    let md = &app.preview.markdown;
    let (Some(parsed), Some(rope)) = (
        md.parsed.as_ref(),
        app.editor.active().and_then(|doc| doc.text().rope()),
    ) else {
        return false;
    };
    parsed.ranges.first().is_some_and(|range| range.start == 0)
        && matches!(
            parsed.blocks.first(),
            Some(Block::Table(..) | Block::Code { .. })
        )
        && rope.bytes().take(3).eq(*b"---")
}
