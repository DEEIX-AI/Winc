//! Markdown 预览中的拖动：鼠标停在块或顶层列表项上时在左侧显示把手，按住上下拖动调整它在
//! 原文中的先后，松开后按新顺序改写原文。
//!
//! 只看预览和分栏预览的右栏都可以拖；分栏左侧是源代码，在那里拖动仍是选择文字。顶层无序
//! 列表和待办列表按单项拖，嵌套列表项随父项一起移动；有序列表也按单项拖，并自动重编号、
//! 调整续行的缩进。docx、EPUB、LaTeX 的预览由转换得到，预览
//! 中的块与原文的位置不对应，因此不能拖动。
//!
//! 拖动期间只改变把手、浮层、插入线和兄弟块的让位，不改动原文；松开时才替换一次原文，因此
//! 撤销一次即可恢复原来的顺序。放下后由 `BlockMotion` / `ItemMotion` 从当前视觉位置继续滑到
//! 新位置（见 `Before::motion` 和 `ItemMotion`）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cpui_render::Rect;
use winc_core::Selection;
use winc_preview::markdown::{Block, ListItems, Markdown, split_list_items};

use crate::app::animation::Clock;
use crate::app::preview::markdown::laid::Laid;
use crate::app::{App, Drag, Focus};

use super::MarkdownPreview;

/// 把手方块的边长（逻辑像素）。
pub(crate) const HANDLE: f32 = 12.0;
/// 行号的右边离正文左边的距离（逻辑像素），把手左边再减去这段。
pub(crate) const NUMBER_GAP: f32 = 12.0;
/// 折叠箭头的大小（逻辑像素），与 `view::preview::markdown` 的箭头一致。
pub(crate) const FOLD: f32 = 8.0;
/// 把手与折叠箭头之间的间距（逻辑像素）。
const HANDLE_FOLD_GAP: f32 = 8.0;
/// 把手相对预留槽位再向左移的距离（逻辑像素）；折叠箭头位置不变，两者之间更松。
pub(crate) const HANDLE_LEFT_OFFSET: f32 = 8.0;
/// 没有行号时折叠箭头与正文之间的间距（逻辑像素）。
const FOLD_GAP: f32 = 8.0;
/// 有行号时把手右缘与行号栏左缘之间保留的间距（逻辑像素）。
const HANDLE_NUMBERS_GAP: f32 = 16.0;
/// 拖动到预览上下边缘以内该距离时开始自动滚动（逻辑像素）。
const EDGE: f32 = 28.0;
/// 鼠标停在边缘之外时每收到一次移动事件最多滚动的距离（逻辑像素）。
const AUTOSCROLL: f32 = 12.0;

/// 拖动期间插入位置指示线的高度（逻辑像素）。
pub(crate) const INDICATOR: f32 = 2.0;

/// 预览中可以拖动的一个单元：一个顶层块，或顶层列表中的一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Element {
    /// 顶层块。
    Block(usize),
    /// 顶层列表中的一项。
    Item { block: usize, item: usize },
}

impl Element {
    /// 单元所在的顶层块序号。
    pub(crate) fn block(self) -> usize {
        match self {
            Element::Block(block) => block,
            Element::Item { block, .. } => block,
        }
    }
}

/// 放置位置：插到第几项之前，等于项数时插到最后。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    /// 顶层块边界。
    Block(usize),
    /// 列表项边界（同一列表内）。
    Item(usize),
}

/// 预览中的拖动状态：鼠标悬停的单元、进行中的拖动、左侧留白的宽度和放下后的过渡。
#[derive(Default)]
pub(crate) struct DragState {
    /// 鼠标下的可拖动单元；只用来判断悬停是否改变、要不要重绘。
    pub(crate) hover: Option<Element>,
    /// 正在拖动的单元。
    pub(crate) session: Option<BlockDrag>,
    /// 拖动放下后等新的解析结果到达再让各块滑到新位置。
    pub(crate) slide_on_parse: bool,
    /// 这次重排动画对应的文档版本；之后再编辑就不再套用。
    pub(crate) slide_revision: Option<u64>,
    /// 行号栏（含与正文的间距）的宽度，绘制时量出，命中测试用。
    pub(crate) numbers: f32,
    /// 顶层列表的拆分结果，按 (解析结果, 文档版本) 缓存，避免鼠标移动时反复复制列表原文。
    list_cache: RefCell<ListCache>,
    /// 最近一次列表项几何，避免悬停和开始拖动时重复计算。
    shape_cache: RefCell<Option<ShapeCache>>,
    /// 松手后、新解析到达前仍然显示在松手位置的浮动卡片。
    pub(crate) drop_card: Option<DropCard>,
}

/// 松手后等待新解析期间保留的浮动卡片。
pub(crate) struct DropCard {
    pub(crate) block: usize,
    pub(crate) element: Element,
    pub(crate) left: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) offset: f32,
    /// 卡片上边缘（窗口坐标）。
    pub(crate) top: f32,
    /// 松手时兄弟块/列表项的让位偏移，等新解析期间保持不动。
    pub(crate) shift: Vec<f32>,
    pub(crate) revision: u64,
}

struct ShapeCache {
    source: usize,
    key: crate::app::preview::markdown::laid::Key,
    block: usize,
    shapes: Rc<Vec<ItemShape>>,
}

#[derive(Default)]
struct ListCache {
    parsed: usize,
    revision: u64,
    items: HashMap<usize, Rc<ListItems>>,
}

impl DragState {
    /// 返回 `block` 的列表拆分结果；同一份解析结果和版本里只复制、拆分一次。
    fn list_items(&self, app: &App, block: usize) -> Option<Rc<ListItems>> {
        let parsed = app.preview.markdown.parsed.as_ref()?;
        let revision = app.editor.active()?.revision;
        let key = std::sync::Arc::as_ptr(parsed) as usize;
        {
            let cache = self.list_cache.borrow();
            if cache.parsed == key
                && cache.revision == revision
                && let Some(items) = cache.items.get(&block)
            {
                return Some(Rc::clone(items));
            }
        }
        let items = Rc::new(load_list_items(app, block)?);
        let mut cache = self.list_cache.borrow_mut();
        if cache.parsed != key || cache.revision != revision {
            cache.parsed = key;
            cache.revision = revision;
            cache.items.clear();
        }
        cache.items.insert(block, Rc::clone(&items));
        Some(items)
    }

    /// 返回 `block` 的列表项几何；排版条件、块和列表拆分都没变时复用上一次的结果。
    fn item_shapes(
        &self,
        laid: &Laid,
        block: usize,
        items: &ListItems,
    ) -> Option<Rc<Vec<ItemShape>>> {
        if let Some(cache) = self.shape_cache.borrow().as_ref()
            && cache.source == laid.source
            && cache.key == laid.key
            && cache.block == block
        {
            return Some(Rc::clone(&cache.shapes));
        }
        let shapes = Rc::new(item_shapes(laid, block, items)?);
        *self.shape_cache.borrow_mut() = Some(ShapeCache {
            source: laid.source,
            key: laid.key,
            block,
            shapes: Rc::clone(&shapes),
        });
        Some(shapes)
    }
}

/// 拖动中兄弟块/列表项的让位偏移，按指数平滑追目标。
pub(crate) struct DragShift {
    current: Vec<f32>,
    target: Vec<f32>,
    at: Instant,
}

impl DragShift {
    fn new() -> DragShift {
        DragShift {
            current: Vec::new(),
            target: Vec::new(),
            at: Instant::now(),
        }
    }

    /// 把当前偏移推向新的目标；返回是否还在移动。
    fn update(&mut self, target: Vec<f32>) -> bool {
        if self.current.len() != target.len() {
            self.current = vec![0.0; target.len()];
        }
        if self.target.len() != target.len() {
            self.target = vec![0.0; target.len()];
        }
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.at)
            .as_secs_f32()
            .min(0.25);
        self.at = now;
        let alpha = 1.0 - (-dt / 0.038).exp();
        let mut moving = false;
        for (i, current) in self.current.iter_mut().enumerate() {
            *current += (target[i] - *current) * alpha;
            if (target[i] - *current).abs() < 0.25 {
                *current = target[i];
            } else {
                moving = true;
            }
        }
        self.target = target;
        moving
    }

    pub(crate) fn at(&self, index: usize) -> f32 {
        self.current.get(index).copied().unwrap_or(0.0)
    }

    fn moving(&self) -> bool {
        self.current
            .iter()
            .zip(&self.target)
            .any(|(current, target)| (target - current).abs() >= 0.25)
    }
}

/// 正在拖动的一个单元。
pub(crate) struct BlockDrag {
    /// 按下的单元。
    pub(crate) element: Element,
    /// 单元所在的顶层块序号。
    pub(crate) block: usize,
    /// 当前的放置位置。
    pub(crate) target: Boundary,
    /// 放置位置在内容中的纵坐标（逻辑像素，相对内容顶部）。
    pub(crate) boundary: f32,
    /// 按下的位置相对单元上边缘的纵向偏移（逻辑像素）。
    pub(crate) grab_y: f32,
    /// 单元在内容坐标中的中心；指针在它上方/下方决定向上还是向下让位。
    pub(crate) center: f32,
    /// 按下的单元的高度、宽度和左边缘（逻辑像素）。
    pub(crate) height: f32,
    pub(crate) width: f32,
    pub(crate) left: f32,
    /// 单元上边缘相对顶层块上边缘的偏移（逻辑像素）；列表项浮层按它平移块内容。
    pub(crate) offset: f32,
    /// 抬起阴影、浮层出现的过渡；动态效果关闭时为 `None`。
    pub(crate) lift: Option<Clock>,
    /// 插入位置指示线滑动到新位置的过渡；尚未移动过时为 `None`。
    pub(crate) indicator: Option<Indicator>,
    /// 兄弟块/列表项的让位偏移。
    pub(crate) shift: DragShift,
    /// 按下时文档的标识与版本号：松开时两者任一改变就不再改写原文。
    doc: u64,
    revision: u64,
}

/// 插入位置指示线从一处滑到另一处的过渡。
pub(crate) struct Indicator {
    from: f32,
    to: f32,
    clock: Option<Clock>,
}

impl Indicator {
    /// 指示线当前的纵坐标（逻辑像素，相对内容顶部）。
    fn y(&self) -> f32 {
        self.clock
            .map_or(self.to, |clock| clock.between(self.from, self.to))
    }
}

impl BlockDrag {
    /// 返回拖动浮层当前的抬起进度：0 表示刚按下，1 表示已经抬起。
    pub(crate) fn lifted(&self) -> f32 {
        self.lift.map_or(1.0, |clock| clock.progress())
    }

    /// 返回插入位置指示线当前的纵坐标（逻辑像素，相对内容顶部）。
    pub(crate) fn boundary(&self) -> f32 {
        self.indicator.as_ref().map_or(self.boundary, Indicator::y)
    }

    /// 拖动过程中是否仍有动画在播放。
    pub(crate) fn animating(&self) -> bool {
        self.lift.is_some_and(|clock| !clock.done())
            || self
                .indicator
                .as_ref()
                .and_then(|indicator| indicator.clock)
                .is_some_and(|clock| !clock.done())
            || self.shift.moving()
    }
}

/// 拖动结束时各顶层块按旧序号记录的视觉偏移，经 `Moves.matched` 对应到重排后的块。
pub(crate) struct ReleasedShift {
    /// 按旧排版块序号索引的视觉偏移。
    pub(crate) offsets: Vec<f32>,
    /// 被拖动的旧块序号和它在新排版中的序号；内容相同的块靠它纠正匹配。
    pub(crate) dragged: Option<(usize, usize)>,
    /// 松开时文档的版本号；之后的编辑不再套用这些偏移。
    pub(crate) revision: u64,
}

/// 列表项放下后的滑动：旧顺序各项的上边缘、新顺序到旧顺序的映射和时钟。
pub(crate) struct ItemMotion {
    pub(crate) block: usize,
    revision: u64,
    old_tops: Vec<f32>,
    order: Vec<usize>,
    /// 新解析结果到达后才启动；等待期间不移动。
    clock: Option<Clock>,
    /// 创建时刻；等待解析超过一秒就放弃，避免永远占着动画帧。
    created: Instant,
}

impl ItemMotion {
    /// 过渡是否已经结束；还没开始（等待新解析结果）时不算结束。
    pub(crate) fn finished(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|clock| clock.progress() >= 1.0)
    }
}

/// 一帧里各列表项的纵向偏移，以及用来把绘制项归到某一项的形状。
pub(crate) struct ItemSlide {
    offsets: Vec<f32>,
    shapes: Rc<Vec<ItemShape>>,
}

impl ItemSlide {
    /// 各列表项这一帧的纵向偏移（测试用）。
    #[cfg(test)]
    pub(crate) fn offsets(&self) -> &[f32] {
        &self.offsets
    }

    /// 返回内容坐标 `y` 所在列表项这一帧的纵向偏移；找不到时为 0。
    pub(crate) fn offset(&self, y: f32) -> f32 {
        let i = self
            .shapes
            .partition_point(|shape| shape.top <= y)
            .saturating_sub(1)
            .min(self.offsets.len().saturating_sub(1));
        self.offsets.get(i).copied().unwrap_or(0.0)
    }
}

/// 一个拖动单元在屏幕上的几何（逻辑像素）。
struct ElementGeometry {
    /// 上边缘（窗口坐标，含过渡动画）。
    top: f32,
    /// 第一行文字中心（窗口坐标）。
    center: f32,
    height: f32,
    /// 浮层宽度。
    width: f32,
    /// 相对所在块内容顶部的纵向偏移。
    offset: f32,
    /// 内容坐标中的起点。
    boundary: f32,
}

/// 让 `MarkdownPreview` 每帧判断鼠标下的单元并绘制把手。
impl MarkdownPreview {
    /// 返回窗口坐标 `(x, y)` 处可以拖动的单元。
    pub(crate) fn element_at(&self, app: &App, x: f32, y: f32) -> Option<Element> {
        if self.drag.session.is_some() || !shows_handles(app) {
            return None;
        }
        let laid = self.layout.as_ref()?;
        let area = self.area;
        if !area.contains(x, y)
            || self.toc_list.card.contains(x, y)
            || self.toc_list.track.contains(x, y)
        {
            return None;
        }
        // 左侧留白中的把手、标题折叠箭头和行号也可以按下；右侧只到正文一栏的右边缘。
        let left = handle_left(laid.left, self.drag.numbers).max(area.x);
        if x < left || x > laid.left + laid.width() {
            return None;
        }
        let top = area.y + laid.pad - crate::layout::drawn_scroll(self.scroll());
        let y = y - top;
        // 按块的上边缘划分整个纵向范围，块之间的间距归上一块：鼠标从一段移向下一段时把手
        // 不会消失。
        let block = laid.tops.partition_point(|&top| top <= y).saturating_sub(1);
        laid.blocks.get(block)?;
        // 顶层无序列表先按项找：一项一个把手，拖动时只换这一项。
        let element = match self.drag.list_items(app, block) {
            Some(items) => {
                let shapes = self.drag.item_shapes(laid, block, &items)?;
                let item = shapes
                    .partition_point(|shape| shape.top <= y)
                    .saturating_sub(1)
                    .min(shapes.len().saturating_sub(1));
                Element::Item { block, item }
            }
            None => Element::Block(block),
        };
        element_movable(app, element).then_some(element)
    }

    /// 返回单元在屏幕上的几何：上边缘、第一行中心、高度、浮层宽度、相对块顶的偏移和
    /// 内容坐标中的起点。命中框、浮层和放下位置都用这一份结果。
    fn geometry(&self, app: &App, element: Element) -> Option<ElementGeometry> {
        let laid = self.layout.as_ref()?;
        let block = element.block();
        let block_top = self.block_top(block)?;
        match element {
            Element::Block(block) => {
                let entry = laid.blocks.get(block)?;
                Some(ElementGeometry {
                    top: block_top,
                    center: first_line_center(&entry.block, block_top),
                    height: entry.block.height,
                    width: ghost_width(&entry.block, laid.width()),
                    offset: 0.0,
                    boundary: laid.tops[block],
                })
            }
            Element::Item { block, item } => {
                let items = self.drag.list_items(app, block)?;
                let shapes = self.drag.item_shapes(laid, block, &items)?;
                let shape = shapes.get(item)?;
                let offset = shape.top - laid.tops[block];
                Some(ElementGeometry {
                    top: block_top + offset,
                    center: block_top + shape.center - laid.tops[block],
                    height: (shape.bottom - shape.top).max(1.0),
                    width: shape.width,
                    offset,
                    boundary: shape.top,
                })
            }
        }
    }

    /// 返回把手在当前帧的矩形（窗口坐标）；单元不可拖动时为 `None`。
    pub(crate) fn handle_rect(&self, app: &App, element: Element) -> Option<Rect> {
        if !element_movable(app, element) {
            return None;
        }
        let center = self.geometry(app, element)?.center;
        let x = handle_left(self.layout.as_ref()?.left, self.drag.numbers).max(self.area.x);
        Some(Rect::new(x, center - 10.0, HANDLE, 20.0))
    }

    /// 返回列表项放下后的滑动；动画结束、块不匹配、文档已变或新解析还没到时不返回。
    pub(crate) fn item_slide(&self, app: &App, block: usize) -> Option<ItemSlide> {
        let motion = self.item_motion.as_ref()?;
        let clock = motion.clock.as_ref()?;
        if motion.block != block || clock.progress() >= 1.0 {
            return None;
        }
        let revision = app.editor.active()?.revision;
        if revision != motion.revision || self.checked != Some(revision) {
            return None;
        }
        let laid = self.layout.as_ref()?;
        let items = self.drag.list_items(app, block)?;
        let shapes = self.drag.item_shapes(laid, block, &items)?;
        if shapes.len() != motion.order.len() || motion.old_tops.len() != motion.order.len() {
            return None;
        }
        let left = 1.0 - clock.progress();
        let offsets = (0..shapes.len())
            .map(|i| (motion.old_tops[motion.order[i]] - shapes[i].top) * left)
            .collect();
        Some(ItemSlide { offsets, shapes })
    }

    /// 新排版到达或文档又变了以后，撤掉等待中的浮动卡片。
    pub(crate) fn clear_drop_card(&mut self, revision: u64) {
        let done =
            self.drag.drop_card.as_ref().is_some_and(|card| {
                card.revision != revision || self.checked == Some(card.revision)
            });
        if done {
            self.drag.drop_card = None;
        }
    }

    /// 返回拖动中的列表项让位偏移；不在拖列表项时返回 `None`。
    pub(crate) fn drag_item_slide(&self, app: &App) -> Option<(usize, ItemSlide)> {
        let (block, offsets) = if let Some(drag) = self.drag.session.as_ref() {
            let Element::Item { block, .. } = drag.element else {
                return None;
            };
            (block, drag.shift.current.clone())
        } else {
            let card = self.drag.drop_card.as_ref()?;
            let Element::Item { block, .. } = card.element else {
                return None;
            };
            (block, card.shift.clone())
        };
        let laid = self.layout.as_ref()?;
        let items = self.drag.list_items(app, block)?;
        let shapes = self.drag.item_shapes(laid, block, &items)?;
        if shapes.len() != offsets.len() {
            return None;
        }
        Some((block, ItemSlide { offsets, shapes }))
    }

    /// 拖动中的列表项：它的原文字节范围（相对顶层块）和内容坐标中的上下边缘。
    ///
    /// 绘制时跳过这一项自己的原文，给让位出来的兄弟项腾位置；只靠一块原位遮罩会把
    /// 滑进旧位置的兄弟项一起盖住（拖动向上时尤其明显）。
    pub(crate) fn dragged_item(
        &self,
        app: &App,
        block: usize,
    ) -> Option<(std::ops::Range<usize>, f32, f32)> {
        let (dragged_block, item) = if let Some(drag) = self.drag.session.as_ref() {
            let Element::Item {
                block: dragged_block,
                item,
            } = drag.element
            else {
                return None;
            };
            (dragged_block, item)
        } else {
            let card = self.drag.drop_card.as_ref()?;
            let Element::Item {
                block: dragged_block,
                item,
            } = card.element
            else {
                return None;
            };
            (dragged_block, item)
        };
        if dragged_block != block {
            return None;
        }
        let laid = self.layout.as_ref()?;
        let items = self.drag.list_items(app, block)?;
        let range = items.ranges.get(item)?.clone();
        let shapes = self.drag.item_shapes(laid, block, &items)?;
        let shape = shapes.get(item)?;
        Some((range, shape.top, shape.bottom))
    }

    /// 返回第 `block` 块在屏幕上的上边缘（窗口坐标，逻辑像素，含过渡动画的偏移）。
    pub(crate) fn block_top(&self, block: usize) -> Option<f32> {
        let laid = self.layout.as_ref()?;
        let y = self.area.y + laid.pad - crate::layout::drawn_scroll(self.scroll())
            + laid.tops.get(block)?;
        Some(y + self.motion.as_ref().map_or(0.0, |m| m.offset(block)))
    }
}

/// 返回顶层无序列表或有序列表的项范围（相对顶层块，字节）。
///
/// 只处理顶层列表：嵌套在列表项里的列表不是顶层块。有序列表的起始编号和分隔符必须与解析
/// 结果一致；标记位数变化由 [`winc_preview::markdown::OrderedList::renumber`] 调整续行缩进。
fn load_list_items(app: &App, block: usize) -> Option<ListItems> {
    let md = &app.preview.markdown;
    let parsed = md.parsed.as_ref()?;
    let (start, items) = match parsed.blocks.get(block)? {
        Block::List(None, items) => (None, items),
        Block::List(Some(start), items) => (Some(*start), items),
        _ => return None,
    };
    if items.len() < 2 {
        return None;
    }
    let laid = md.layout.as_ref()?;
    // 刚放下块时原文已经改了，但新解析/排版可能还没到：旧范围落在新文字里可能切在 UTF-8
    // 字符中间。排版不是按当前解析结果算的就先不认列表，等这一帧排好再认。
    if laid.source != md.parse_serial {
        return None;
    }
    let range = laid.ranges.get(block)?;
    let rope = app.editor.active()?.text().rope()?;
    let start_byte = range.start.min(rope.len_bytes());
    let end_byte = range.end.min(rope.len_bytes()).max(start_byte);
    // 防御：即使排版来源相同，范围也应落在字符边界上；不落在就当没有列表，不让绘制路径 panic。
    let text = rope.get_byte_slice(start_byte..end_byte)?.to_string();
    let list = split_list_items(&text)?;
    if list.ranges.len() != items.len() {
        return None;
    }
    match (start, &list.ordered) {
        (None, None) => {}
        (Some(start), Some(ordered)) if ordered.start() == start => {}
        _ => return None,
    }
    Some(list)
}

/// 返回拖动浮层的宽度（逻辑像素）：块中文字、图片和线条到达的最右处，限制在正文一栏内。
///
/// 只用内容本身定宽：代码块、引用的底色和表格的外框会铺满一栏，按它们定宽会使标题的浮层
/// 和整栏一样宽，插入位置指示线被完全盖住。
fn ghost_width(block: &winc_typeset::LaidBlock, max: f32) -> f32 {
    use winc_typeset::Item;
    let mut width: f32 = 0.0;
    for item in &block.items {
        let right = match item {
            Item::Text {
                x,
                text,
                font,
                span,
                ..
            } => match span {
                Some(span) => x + span.advances.iter().sum::<f32>(),
                // 没有字宽记录的文字（图里的标注等）按字号的六成估计。
                None => x + font.size * 0.6 * text.chars().count() as f32,
            },
            Item::Image { rect, .. } | Item::Path { rect, .. } | Item::Polygon { rect, .. } => {
                rect.right()
            }
            _ => 0.0,
        };
        width = width.max(right);
    }
    width.clamp(64.0, max)
}

/// 返回第 `block` 块第一行文字的中心（窗口坐标，逻辑像素）。
pub(crate) fn first_line_center(block: &winc_typeset::LaidBlock, top: f32) -> f32 {
    use winc_typeset::Item;
    for item in &block.items {
        if let Item::Text { baseline, font, .. } = item {
            return top + baseline - font.size * 0.35;
        }
    }
    top + (block.height / 2.0).min(HANDLE / 2.0)
}

/// 第 `block` 块在排版和解析结果中是否可以作为一个整体移动。
///
/// 看不见的块（收起的一节里的块、脚注前生成的分隔线）没有可拖动的画面；脚注在预览中排在
/// 文末，原文位置却可能在正文之间，不作为一个块拖动。
fn block_movable(laid: &Laid, parsed: &Markdown, len: usize, block: usize) -> bool {
    let (Some(entry), Some(range)) = (laid.blocks.get(block), laid.ranges.get(block)) else {
        return false;
    };
    entry.block.height > 0.0
        && !range.is_empty()
        && range.end <= len
        && !matches!(parsed.blocks.get(block), Some(Block::Footnote(..)))
}

/// 单元是否可以拖动：只看预览、文档可以编辑，并且解析结果与当前文字一致。
pub(crate) fn element_movable(app: &App, element: Element) -> bool {
    let md = &app.preview.markdown;
    if !drag_ready(app) {
        return false;
    }
    let (Some(parsed), Some(laid)) = (md.parsed.as_ref(), md.layout.as_ref()) else {
        return false;
    };
    let len = app
        .editor
        .active()
        .and_then(|doc| doc.text().rope())
        .map_or(0, ropey::Rope::len_bytes);
    let block = element.block();
    if laid.blocks.get(block).is_none() || !block_movable(laid, parsed, len, block) {
        return false;
    }
    match element {
        Element::Block(_) => true,
        Element::Item { block, item } => app
            .preview
            .markdown
            .drag
            .list_items(app, block)
            .is_some_and(|list| list.ranges.get(item).is_some_and(|range| !range.is_empty())),
    }
}

/// 预览中是否显示拖动把手：预览显示在编辑区（只看预览或分栏右栏），文档可以编辑。
///
/// 解析结果是否已经跟上当前文字由 [`drag_ready`] 判断；留白始终按这个函数保留。
///
/// docx、EPUB 的预览和阅读标签由转换得到，预览中的块与原文位置不对应；LaTeX 预览编译成 PDF，
/// 也不在这里。
pub(crate) fn shows_handles(app: &App) -> bool {
    if app.config.markdown_drag != Some(true) {
        return false;
    }
    let Some(doc) = app.editor.active() else {
        return false;
    };
    doc.edits_text()
        && doc.readonly_reason().is_none()
        && doc.unsupported.is_none()
        && doc.reading.is_none()
        && app.preview.markdown.lines.is_none()
}

/// 预览中的块现在是否可以拖动：显示把手，并且解析结果与当前文字一致。
///
/// 解析或排版进行中时把手暂时不响应，但左侧留白仍按 [`shows_handles`] 保留，文字不会左右跳动。
pub(crate) fn drag_ready(app: &App) -> bool {
    let Some(doc) = app.editor.active() else {
        return false;
    };
    let md = &app.preview.markdown;
    shows_handles(app) && md.checked == Some(doc.revision) && md.pending.is_none()
}

/// 返回正文左侧为把手、折叠箭头和行号栏预留的宽度（逻辑像素）。
///
/// 从左到右依次是把手、折叠箭头、行号栏、正文；没有行号时行号栏的宽度为 0，箭头与正文之间
/// 留 8 像素。`numbers` 是行号栏（含它与正文的间距）的宽度，绘制行号时由 `number_room` 量出。
/// 把手本身还向左偏 [`HANDLE_LEFT_OFFSET`]，占用预留槽位之外的外层留白。
pub(crate) fn gutter(numbers: f32) -> f32 {
    if numbers > 0.0 {
        // 把手右缘到行号栏左缘留 16 像素；把手比预留槽位再左偏 8 像素，所以槽位宽度是
        // 行号栏宽度 + 16 + 把手宽 12 - 左偏 8。
        numbers + HANDLE_NUMBERS_GAP + HANDLE - HANDLE_LEFT_OFFSET
    } else {
        // 没有行号栏时箭头与正文之间留 8 像素。
        HANDLE + HANDLE_FOLD_GAP + FOLD + FOLD_GAP
    }
}

/// 返回把手方块的左边（窗口坐标）。正文左边是 `left`，左侧预留由 [`gutter`] 给出；
/// 把手再向左让出 [`HANDLE_LEFT_OFFSET`]，与折叠箭头拉开距离。
pub(crate) fn handle_left(left: f32, numbers: f32) -> f32 {
    left - gutter(numbers) - HANDLE_LEFT_OFFSET
}

/// 返回折叠箭头的中心横坐标（窗口坐标），与绘制时的位置一致。
///
/// 有行号栏时箭头紧贴行号栏左侧；没有行号栏时放在正文左边缘与预留左边界之间。
pub(crate) fn fold_center(left: f32, has_numbers: bool, pad: f32) -> f32 {
    if has_numbers {
        left - NUMBER_GAP - FOLD / 2.0
    } else {
        left - pad / 2.0
    }
}

impl App {
    /// 更新鼠标下的可拖动单元；单元改变时重绘，以显示或隐藏把手。
    pub(crate) fn track_markdown_drag_hover(&mut self, x: f32, y: f32) {
        let element = self.preview.markdown.element_at(self, x, y);
        if element != self.preview.markdown.drag.hover {
            self.preview.markdown.drag.hover = element;
            self.request_redraw();
        }
    }

    /// 返回鼠标位置（或指定位置）的把手所在的单元。
    pub(crate) fn markdown_handle_at(&self, x: f32, y: f32) -> Option<Element> {
        let md = &self.preview.markdown;
        // 按当前指针位置重新找单元：解析结果、块序号可能在上一次鼠标移动之后变了。
        let element = md.element_at(self, x, y)?;
        md.handle_rect(self, element)
            .filter(|rect| rect.contains(x, y))
            .map(|_| element)
    }

    /// 新的解析与排版到达后启动列表项滑动；等不到新排版时保持不动。
    pub(crate) fn start_item_motion(&mut self) {
        let revision = self.editor.active().map(|doc| doc.revision);
        let stale = self
            .preview
            .markdown
            .item_motion
            .as_ref()
            .is_some_and(|motion| {
                revision != Some(motion.revision)
                    || (motion.clock.is_none() && motion.created.elapsed() > Duration::from_secs(1))
            });
        if stale {
            self.preview.markdown.item_motion = None;
            return;
        }
        let ready = self
            .preview
            .markdown
            .item_motion
            .as_ref()
            .is_some_and(|motion| {
                motion.clock.is_none() && self.preview.markdown.checked == Some(motion.revision)
            });
        if !ready {
            return;
        }
        let Some(clock) = self.drag_clock(crate::app::animation::ITEM_SLIDE) else {
            self.preview.markdown.item_motion = None;
            self.preview.markdown.drag_release = None;
            return;
        };
        if let Some(motion) = self.preview.markdown.item_motion.as_mut() {
            motion.clock = Some(clock);
        }
    }

    /// 更新拖动中的让位偏移：ArcMD 式指数平滑，目标随最近边界变化。
    pub(crate) fn update_markdown_drag_shift(&mut self) {
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return;
        };
        let Some(session) = self.preview.markdown.drag.session.as_ref() else {
            return;
        };
        let (element, block, target, height) = (
            session.element,
            session.block,
            session.target,
            session.height,
        );
        let (unit, count, step) = match element {
            Element::Block(_) => {
                let slot = laid
                    .tops
                    .get(block + 1)
                    .map_or(height, |next| next - laid.tops[block]);
                (block, laid.blocks.len(), slot)
            }
            Element::Item { item, .. } => {
                let count = self
                    .preview
                    .markdown
                    .drag
                    .list_items(self, block)
                    .map_or(0, |list| list.ranges.len());
                (item, count, height)
            }
        };
        let mut target_offsets = vec![0.0; count];
        let slot = match target {
            Boundary::Block(slot) | Boundary::Item(slot) => slot,
        };
        if slot > unit {
            for offset in target_offsets
                .iter_mut()
                .take(slot.min(count))
                .skip(unit + 1)
            {
                *offset = -step;
            }
        } else if slot < unit {
            for offset in target_offsets.iter_mut().take(unit).skip(slot) {
                *offset = step;
            }
        }
        let Some(session) = self.preview.markdown.drag.session.as_mut() else {
            return;
        };
        if session.shift.update(target_offsets) {
            self.request_redraw();
        }
    }

    /// 在把手处按下：开始拖动，不改动原文。
    ///
    /// 文档在按下之后、松开之前被修改过时，松开时不再改写原文，避免把旧的位置用到新文字上。
    pub(in crate::app) fn start_markdown_drag(&mut self, element: Element, y: f32) {
        if !element_movable(self, element) {
            return;
        }
        // 新的拖动从当前画面重新开始：上一次放下的滑动、等待解析的浮动卡片和滑入标记都
        // 不能叠到这一次上。
        self.preview.markdown.motion = None;
        self.preview.markdown.item_motion = None;
        self.preview.markdown.drag_release = None;
        self.preview.markdown.drag.drop_card = None;
        self.preview.markdown.drag.slide_on_parse = false;
        self.preview.markdown.drag.slide_revision = None;
        let block = element.block();
        let Some(geometry) = self.preview.markdown.geometry(self, element) else {
            return;
        };
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return;
        };
        let Some(doc) = self.editor.active() else {
            return;
        };
        let (doc, revision) = (doc.id.0, doc.revision);
        let top = geometry.top;
        let height = geometry.height;
        let width = geometry.width;
        let offset = geometry.offset;
        let boundary = geometry.boundary;
        let center = geometry.center
            - (self.preview.markdown.area.y + laid.pad
                - crate::layout::drawn_scroll(self.preview.markdown.scroll()));
        let left = laid.left;
        let target = match element {
            Element::Block(block) => Boundary::Block(block),
            Element::Item { item, .. } => Boundary::Item(item),
        };
        let lift = self.drag_clock(crate::app::animation::BLOCK_LIFT);
        self.preview.markdown.drag.session = Some(BlockDrag {
            element,
            block,
            target,
            boundary,
            grab_y: (y - top).clamp(0.0, height.max(1.0)),
            center,
            height,
            width,
            left,
            offset,
            lift,
            indicator: None,
            shift: DragShift::new(),
            doc,
            revision,
        });
        self.preview.markdown.drag.hover = Some(element);
        self.preview.markdown.selection = None;
        self.focus = Focus::Editor;
        self.pointer.drag = Drag::MarkdownBlock;
        self.request_redraw();
    }

    /// 拖动中的鼠标移动：按鼠标的纵坐标更新放置位置，靠近上下边缘时自动滚动。
    pub(in crate::app) fn drag_markdown_block(&mut self, y: f32) {
        if self.preview.markdown.drag.session.is_none() {
            return;
        }
        // 浮层跟着指针走，不能只在跨过插入边界时才重绘：指针在同一个块内移动时也要重画。
        self.request_redraw();
        let area = self.preview.markdown.area;
        let scroll = self.preview.markdown.scroll();
        let delta = if y < area.y + EDGE {
            -AUTOSCROLL * ((area.y + EDGE - y) / EDGE).clamp(0.0, 1.0)
        } else if y > area.bottom() - EDGE {
            AUTOSCROLL * ((y - area.bottom() + EDGE) / EDGE).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if delta != 0.0 {
            self.scroll_markdown_to(scroll + delta);
        }
        let Some(laid) = self.preview.markdown.layout.as_ref() else {
            return;
        };
        let content =
            y - (area.y + laid.pad - crate::layout::drawn_scroll(self.preview.markdown.scroll()));
        let Some(parsed) = self.preview.markdown.parsed.as_ref() else {
            return;
        };
        let Some(drag) = self.preview.markdown.drag.session.as_ref() else {
            return;
        };
        let element = drag.element;
        // 指针在单元原中心上方就按“向上拖”算，越过的目标块立刻让位；下方按“向下拖”算。
        let up = content < drag.center;
        let (target, boundary) = match element {
            Element::Block(_) => {
                let target = boundary_near(laid, parsed, content, up);
                (Boundary::Block(target), boundary_y(laid, target))
            }
            Element::Item { block, .. } => {
                let Some((slot, boundary)) = item_target(self, block, content, up) else {
                    return;
                };
                (Boundary::Item(slot), boundary)
            }
        };
        let effective = boundary_effective(self, parsed, element, target);
        let same = drag.target == target && drag.indicator.is_some() == effective;
        if same {
            return;
        }
        let from = drag.boundary();
        let clock = effective.then(|| self.drag_clock(crate::app::animation::BLOCK_SLIDE));
        let Some(drag) = self.preview.markdown.drag.session.as_mut() else {
            return;
        };
        drag.target = target;
        drag.boundary = boundary;
        drag.indicator = clock.map(|clock| Indicator {
            from,
            to: boundary,
            clock,
        });
        self.request_redraw();
    }

    /// 放弃正在进行的块拖动：不改动原文，恢复鼠标状态。
    pub(crate) fn cancel_markdown_block_drag(&mut self) {
        if self.preview.markdown.drag.session.take().is_some() {
            self.preview.markdown.drag.hover = None;
            self.request_redraw();
        }
        if self.pointer.drag == Drag::MarkdownBlock {
            self.pointer.drag = Drag::None;
            self.refresh_pointer_hover();
            self.update_cursor_icon();
        }
    }

    /// 松开鼠标：把块移到当前的放置位置，改写一次原文。返回是否改动了原文。
    pub(in crate::app) fn finish_markdown_block_drag(&mut self) -> bool {
        let Some(drag) = self.preview.markdown.drag.session.take() else {
            return false;
        };
        self.preview.markdown.drag.hover = None;
        self.request_redraw();
        // 拖到预览外面（侧栏、顶栏）再松开：不移动。上下各放宽一个自动滚动的距离，停在边缘
        // 之外一点时仍按最近的边界放下。
        let area = self.preview.markdown.area;
        let (mx, my) = self.mouse();
        if mx < area.x || mx > area.right() || !(area.y - EDGE..=area.bottom() + EDGE).contains(&my)
        {
            return false;
        }
        let Some(doc) = self.editor.active() else {
            return false;
        };
        if doc.id.0 != drag.doc || doc.revision != drag.revision {
            return false;
        }
        let (Some(laid), Some(rope)) = (self.preview.markdown.layout.as_ref(), doc.text().rope())
        else {
            return false;
        };
        let Some(parsed) = self.preview.markdown.parsed.as_ref() else {
            return false;
        };
        // 解析已经换了、排版还没跟上：旧范围不能拿来改写新文字（会切在 UTF-8 字符中间）。
        if laid.source != self.preview.markdown.parse_serial {
            return false;
        }
        if !boundary_effective(self, parsed, drag.element, drag.target) {
            return false;
        }
        let origin =
            area.y + laid.pad - crate::layout::drawn_scroll(self.preview.markdown.scroll());
        let card_top = (my - drag.grab_y - 3.0 * drag.lifted()).clamp(
            area.y + 6.0,
            (area.bottom() - drag.height - 6.0).max(area.y + 6.0),
        );
        let visual_top = card_top - origin;
        let mut drop_card = DropCard {
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
        let mut release = None;
        if matches!(drag.element, Element::Block(_)) {
            let mut offsets = drag.shift.current.clone();
            offsets.resize(laid.blocks.len(), 0.0);
            offsets[drag.block] = visual_top - laid.tops[drag.block];
            let dragged = match drag.target {
                Boundary::Block(target) => Some((
                    drag.block,
                    if target > drag.block {
                        target - 1
                    } else {
                        target
                    },
                )),
                Boundary::Item(_) => None,
            };
            release = Some(ReleasedShift {
                offsets,
                dragged,
                revision: 0,
            });
        }
        let mut item_motion = None;
        let text_move = match drag.element {
            Element::Block(block) => {
                let Boundary::Block(target) = drag.target else {
                    return false;
                };
                let Some(moved) = moved_text_with(
                    rope.len_bytes(),
                    &laid.ranges,
                    block,
                    target,
                    doc.line_ending.as_str(),
                    |range| rope.byte_slice(range).to_string(),
                ) else {
                    return false;
                };
                moved
            }
            Element::Item { block, item } => {
                let Boundary::Item(slot) = drag.target else {
                    return false;
                };
                let Some(range) = laid.ranges.get(block) else {
                    return false;
                };
                let len = rope.len_bytes();
                let start = range.start.min(len);
                let end = range.end.min(len).max(start);
                let text = rope.byte_slice(start..end).to_string();
                let Some(list) = self.preview.markdown.drag.list_items(self, block) else {
                    return false;
                };
                let Some(moved) = moved_list_text(&text, start, &list, item, slot) else {
                    return false;
                };
                if let Some(shapes) = self.preview.markdown.drag.item_shapes(laid, block, &list) {
                    let mut tops: Vec<f32> = shapes.iter().map(|shape| shape.top).collect();
                    for (i, top) in tops.iter_mut().enumerate() {
                        *top += drag.shift.at(i);
                    }
                    tops[item] = visual_top;
                    item_motion = Some((block, tops, item_order(item, slot, list.ranges.len())));
                }
                moved
            }
        };
        // 先把旧选区映射到改写后的字节位置，插入后再换成字符位置。被移动的段按段内偏移
        // 跟到新位置，向两头移动都不会落到顶替它的块里。
        let previous = doc.selection;
        let old_chars = rope.len_chars();
        let mapped = (
            text_move.map(rope.char_to_byte(previous.anchor.min(old_chars))),
            text_move.map(rope.char_to_byte(previous.head.min(old_chars))),
        );
        let start = rope.byte_to_char(text_move.span.start.min(rope.len_bytes()));
        let end = rope.byte_to_char(text_move.span.end.min(rope.len_bytes()));
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        doc.collapse_selections();
        doc.selection = Selection::new(start, end);
        self.run(winc_editor::Command::Insert(text_move.text));
        if let Some(doc) = self.editor.active_mut() {
            doc.goal_cell = None;
            if let Some(rope) = doc.text().rope() {
                let at = |byte: usize| rope.byte_to_char(byte.min(rope.len_bytes()));
                doc.selection = Selection::new(at(mapped.0), at(mapped.1));
            }
        }
        // 拖动改写的是文档，不是用户刚移动的光标：同步记录光标位置，避免下一次
        // `follow_live_caret` 把版本变化当成光标移动，把视口拉回原来阅读的光标处。
        if let Some(source) = self.live_caret_source()
            && let Some(doc) = self.editor.active()
        {
            self.preview.markdown.followed = Some((doc.id, source, doc.revision));
        }
        let revision = self.editor.active().map_or(0, |doc| doc.revision);
        drop_card.revision = revision;
        self.preview.markdown.drag.drop_card = Some(drop_card);
        if let Some(mut release) = release {
            release.revision = revision;
            self.preview.markdown.drag_release = Some(release);
        }
        if let Some((block, old_tops, order)) = item_motion {
            let revision = self.editor.active().map_or(0, |doc| doc.revision);
            self.preview.markdown.item_motion = Some(ItemMotion {
                block,
                revision,
                old_tops,
                order,
                clock: None,
                created: Instant::now(),
            });
        }
        self.preview.markdown.goal = None;
        self.preview.markdown.line_end = None;
        self.preview.markdown.selection = None;
        // 顶层块放下后由 `Before::motion` 滑到新位置；列表项用单独的逐项滑动。
        // 记下版本号，之后再输入文字时不把那次重排当成拖动。
        let slide = matches!(drag.element, Element::Block(_));
        self.preview.markdown.drag.slide_on_parse = slide;
        self.preview.markdown.drag.slide_revision = slide.then_some(revision);
        true
    }

    /// 拖动把手的动画时钟。测试中按 [`MarkdownPreview::frozen`] 的已用时间固定，使截取的中间帧
    /// 不随运行测试的机器变化。
    fn drag_clock(&mut self, duration: Duration) -> Option<Clock> {
        #[cfg(test)]
        if let Some(at) = self.preview.markdown.frozen {
            return Some(Clock::frozen(at, duration));
        }
        self.clock(duration)
    }
}

/// 返回列表项边界：距离内容坐标 `y` 最近的项起点，或列表末尾。
fn item_target(app: &App, block: usize, y: f32, up: bool) -> Option<(usize, f32)> {
    let layout = app.preview.markdown.layout.as_ref()?;
    let items = app.preview.markdown.drag.list_items(app, block)?;
    let shapes = app
        .preview
        .markdown
        .drag
        .item_shapes(layout, block, &items)?;
    // 向上拖时，指针进入某一项的下边界就插到它前面；向下拖时进入上边界就插到它后面。
    // 这样 A 拖到 B 上时 B 立即让位，不必等指针越过 B 的中线。
    let slot = if up {
        shapes
            .iter()
            .position(|shape| y < shape.bottom)
            .unwrap_or(shapes.len())
    } else {
        shapes
            .iter()
            .position(|shape| y < shape.top)
            .unwrap_or(shapes.len())
    };
    let end = layout.tops[block] + layout.blocks[block].block.height;
    let boundary = shapes.get(slot).map_or(end, |shape| shape.top);
    Some((slot, boundary))
}

/// 目标边界是否会真正改变顺序：原位和紧邻原位都不算。
fn boundary_effective(app: &App, parsed: &Markdown, element: Element, target: Boundary) -> bool {
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return false;
    };
    match (element, target) {
        (Element::Block(block), Boundary::Block(target)) => {
            let (mut position, mut slot, mut visible) = (None, None, 0);
            for i in 0..laid.blocks.len() {
                if !block_movable(laid, parsed, parsed.len, i) {
                    continue;
                }
                if i == block {
                    position = Some(visible);
                }
                if i == target {
                    slot = Some(visible);
                }
                visible += 1;
            }
            let Some(position) = position else {
                return false;
            };
            let slot = slot.unwrap_or(visible);
            slot != position && slot != position + 1
        }
        (Element::Item { item, .. }, Boundary::Item(slot)) => slot != item && slot != item + 1,
        _ => false,
    }
}

/// 返回距离内容坐标 `y` 最近的块边界序号：插到第几块之前，等于块数时表示插到最后。
///
/// 收起的一节里高度为 0 的块、脚注和脚注前生成的分隔线不算边界：它们要么看不见，要么在
/// 原文中的位置与预览中的顺序不同，放在它们前面的位置会让人以为移到了别处。
fn boundary_near(laid: &Laid, parsed: &Markdown, y: f32, up: bool) -> usize {
    // 向上拖时，指针进入某块的下边界就插到它前面；向下拖时进入上边界就插到它后面。
    for i in 0..laid.blocks.len() {
        if !block_movable(laid, parsed, parsed.len, i) {
            continue;
        }
        let edge = if up {
            laid.tops[i] + laid.blocks[i].block.height
        } else {
            laid.tops[i]
        };
        if y < edge {
            return i;
        }
    }
    laid.blocks.len()
}

/// 返回第 `target` 个边界在内容中的纵坐标（逻辑像素）。
fn boundary_y(laid: &Laid, target: usize) -> f32 {
    laid.tops.get(target).copied().unwrap_or(laid.height)
}

/// 一次拖动改写的原文范围和选区映射信息。
struct TextMove {
    /// 要替换的绝对字节范围。
    span: Range<usize>,
    /// 替换后的文字。
    text: String,
    /// 旧文本中未改动、只换了位置的各段，以及它们在新文本中的绝对字节起点。
    chunks: Vec<(Range<usize>, usize)>,
}

impl TextMove {
    /// 把旧文本中的字节位置映射到改写后的新文本。被移动、搬走的段落按段内偏移映射到新位置；
    /// 范围之外按长度差平移；落在这两者之间的空档里时退到替换范围的开头。
    fn map(&self, byte: usize) -> usize {
        // 两个相邻的段在边界上会同时包含同一个字节：优先把它算作后一段的起点，这样
        // 替换范围开头的光标不会跟着被拖走的段跑到另一边。
        if let Some((_, start)) = self.chunks.iter().find(|(range, _)| byte == range.start) {
            return *start;
        }
        if let Some((range, start)) = self
            .chunks
            .iter()
            .find(|(range, _)| byte > range.start && byte <= range.end)
        {
            return start + (byte - range.start);
        }
        if byte < self.span.start {
            return byte;
        }
        if byte >= self.span.end {
            let delta = self.text.len() as isize - self.span.len() as isize;
            return (byte as isize + delta).max(0) as usize;
        }
        self.span.start
    }
}

/// 计算把第 `block` 块移到放置位置 `target` 后，需要替换的原文范围和新文字。
///
/// `ranges` 是各顶层块在原文中的字节范围，`eol` 是文档的行尾。块末尾原有的空行属于分隔，
/// 移走时由目标位置重新给出：移到另一块之前时保留一个空行，避免与相邻块合成一段；移到文末时
/// 只保留一个行尾。返回 `None` 表示不需要改动（放在原位、紧邻原位或参数无效）。
#[cfg(test)]
fn moved_text(
    source: &str,
    ranges: &[Range<usize>],
    block: usize,
    target: usize,
    eol: &str,
) -> Option<TextMove> {
    moved_text_with(source.len(), ranges, block, target, eol, |range| {
        source[range].to_owned()
    })
}

/// [`moved_text`] 的通用形式：只按需要取原文片段，避免为了移动一块而复制整篇文档。
fn moved_text_with<F>(
    len: usize,
    ranges: &[Range<usize>],
    block: usize,
    target: usize,
    eol: &str,
    mut slice: F,
) -> Option<TextMove>
where
    F: FnMut(Range<usize>) -> String,
{
    if block >= ranges.len() || target > ranges.len() || target == block || target == block + 1 {
        return None;
    }
    let start = ranges[block].start.min(len);
    // 下一块按原文中的位置找：脚注在解析结果中排在最后，范围可能落在前面的正文之间。
    let end = ranges
        .iter()
        .map(|range| range.start.min(len))
        .filter(|&at| at > start)
        .min()
        .unwrap_or(len);
    if start >= end {
        return None;
    }
    let destination = ranges.get(target).map_or(len, |range| range.start.min(len));
    if destination >= end {
        // 向下移动：中间各块整体上移，被拖的块放到它们之后。中间部分的末尾统一补成
        // 一个空行，否则块与块之间只剩一个换行，会被 Markdown 当成同一段（文末也一样）。
        let mut gap = slice(end..destination);
        let trimmed = gap.trim_end_matches(['\n', '\r']).len();
        gap.truncate(trimmed);
        gap.push_str(eol);
        gap.push_str(eol);
        let moved = separated(&slice(start..end), eol, destination < len);
        let moved_start = start + gap.len();
        let chunks = vec![(start..end, moved_start), (end..destination, start)];
        return Some(TextMove {
            span: start..destination,
            text: format!("{gap}{moved}"),
            chunks,
        });
    }
    if destination < start {
        let gap = slice(destination..start);
        let moved = separated(&slice(start..end), eol, true);
        let gap_start = destination + moved.len();
        let chunks = vec![(start..end, destination), (destination..start, gap_start)];
        return Some(TextMove {
            span: destination..end,
            text: format!("{moved}{gap}"),
            chunks,
        });
    }
    None
}

/// 返回把第 `item` 项移到第 `slot` 项之前后的旧项顺序：新顺序第 i 项来自旧的第 `order[i]` 项。
fn item_order(item: usize, slot: usize, count: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    order.remove(item);
    let insert = if slot > item { slot - 1 } else { slot };
    order.insert(insert, item);
    order
}

/// 把列表的第 `item` 项移到第 `slot` 项之前，返回整段列表的新文字和选区映射。
///
/// 列表项范围的末尾可能带着列表后的空行；这里把每项拆成正文和末尾的分隔，正文按新顺序
/// 排列，分隔仍留在原来的槽位，因此紧凑/松散列表的空行和列表与后文之间的空行都不变。
/// 有序列表在正文的标记上重新编号，位数变化时同步调整续行缩进。
fn moved_list_text(
    source: &str,
    base: usize,
    list: &ListItems,
    item: usize,
    slot: usize,
) -> Option<TextMove> {
    let ranges = &list.ranges;
    let count = ranges.len();
    if item >= count || slot > count || slot == item || slot == item + 1 {
        return None;
    }
    let order = item_order(item, slot, count);
    let span_start = base + ranges[0].start;
    let mut text = String::with_capacity(source.len());
    let mut chunks = Vec::with_capacity(count * 2);
    for (index, &old) in order.iter().enumerate() {
        let old_range = ranges[old].clone();
        let body_end = item_body_end(&source[old_range.clone()]);
        let body = &source[old_range.start..old_range.start + body_end];
        let body = match &list.ordered {
            Some(ordered) => ordered.renumber(body, index)?,
            None => body.to_owned(),
        };
        let body_start = span_start + text.len();
        text.push_str(&body);
        chunks.push((
            base + old_range.start..base + old_range.start + body_end,
            body_start,
        ));
        // 分隔按槽位保留：第 `index` 个槽位沿用原第 `index` 项末尾的空行。
        let slot_range = ranges[index].clone();
        let slot_body_end = item_body_end(&source[slot_range.clone()]);
        let sep_start = base + slot_range.start + slot_body_end;
        let sep = &source[slot_range.start + slot_body_end..slot_range.end];
        let new_sep = span_start + text.len();
        text.push_str(sep);
        chunks.push((sep_start..base + slot_range.end, new_sep));
    }
    Some(TextMove {
        span: span_start..base + source.len(),
        text,
        chunks,
    })
}

/// 返回块或列表项正文的末尾（去掉末尾的空行）。
fn item_body_end(chunk: &str) -> usize {
    let mut end = chunk.len();
    while end > 0 && matches!(chunk.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    // 最后几行只有空白时同样算作分隔，一并去掉；最后一行末尾的空格是原文的一部分，保留。
    loop {
        let line = chunk[..end].rfind('\n').map_or(0, |at| at + 1);
        if end > line && chunk[line..end].trim().is_empty() {
            end = line;
            while end > 0 && matches!(chunk.as_bytes()[end - 1], b'\n' | b'\r') {
                end -= 1;
            }
        } else {
            break;
        }
    }
    end
}

/// 去掉块末尾的空行，再补上块与下一块之间的空行；移到文末时只补一个行尾。
///
/// 块末尾原有的空行属于分隔，不属于块的内容：移走之后由目标位置重新给出。
fn separated(chunk: &str, eol: &str, blank_line: bool) -> String {
    let end = item_body_end(chunk);
    let mut moved = String::with_capacity(end + 2 * eol.len());
    moved.push_str(&chunk[..end]);
    moved.push_str(eol);
    if blank_line {
        moved.push_str(eol);
    }
    moved
}

mod shapes;
#[cfg(test)]
mod tests;

use shapes::{ItemShape, item_shapes};
