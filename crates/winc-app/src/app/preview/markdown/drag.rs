//! Markdown 预览中的拖动排序：指针停在顶层块或顶层列表项上时，正文左侧显示把手；按住把手
//! 上下拖动调整先后，松开后按新顺序改写一次原文，撤销一次即恢复原来的顺序。
//!
//! 只看预览和分栏预览的右栏支持拖动。嵌套列表项随父项移动，有序列表重新编号。docx、EPUB、
//! LaTeX 的预览由转换得到，块与原文的位置不对应，不支持拖动。拖动期间只改变绘制，放下后
//! 由 `BlockMotion` 和 [`ItemMotion`] 从松手时的位置滑到新位置。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cpui_render::Rect;
use winc_preview::markdown::{Block, ListItems, Markdown, split_list_items};

use crate::app::animation::Clock;
use crate::app::preview::markdown::laid::{Before, Laid};
use crate::app::{App, Drag, Focus};

use super::{MarkdownPreview, Moves};

/// 把手的宽度（逻辑像素，取自调参）：容纳两列圆点及两侧的留白。
pub(crate) const HANDLE: f32 = 12.0;
/// 把手的高度（逻辑像素），与标题折叠箭头的点击范围同高。
pub(crate) const HANDLE_HEIGHT: f32 = 20.0;
/// 原文行号的右边离正文左边的距离（逻辑像素）：与编辑区行号栏和文字之间的间距相近。
pub(crate) const NUMBER_GAP: f32 = 12.0;
/// 标题折叠箭头的大小（逻辑像素）；绘制箭头和计算把手的位置使用同一值。
pub(crate) const FOLD: f32 = 8.0;
/// 把手与折叠箭头之间的间距（逻辑像素），等于 [`FOLD_GAP`]：把手、箭头、正文三者等距。
const HANDLE_FOLD_GAP: f32 = FOLD_GAP;
/// 把手相对预留槽位再向左移的距离（逻辑像素，取自调参）：折叠箭头的位置不变，把手与箭头
/// 之间的空隙加大，指针在两者之间移动时不易按错。
pub(crate) const HANDLE_LEFT_OFFSET: f32 = 8.0;
/// 没有行号时折叠箭头与正文之间的间距（逻辑像素，取自调参）：箭头靠近所在的标题，又不与
/// 标题的首字相接。
const FOLD_GAP: f32 = 8.0;
/// 有行号时把手右缘与行号栏左缘之间的间距（逻辑像素，取自调参）：大于 [`NUMBER_GAP`]，
/// 把手与行号分成两组。
const HANDLE_NUMBERS_GAP: f32 = 16.0;
/// 拖动时指针进入预览上下边缘以内该距离开始自动滚动（逻辑像素，取自调参）：略高于默认
/// 字号下正文的一行（24），指针贴近边缘即开始滚动。
const EDGE: f32 = 28.0;
/// 插入位置指示线的粗细（逻辑像素），与分栏时标出光标所在行的竖条同宽。
pub(crate) const INDICATOR: f32 = 2.0;
/// 浮动卡片抬起后相对指针上移的距离（逻辑像素，取自调参）：配合阴影显出卡片浮起。
pub(crate) const CARD_LIFT: f32 = 3.0;
/// 浮动卡片在内容四周的纵向留白（逻辑像素）；卡片限制在预览内时同样留出这段距离。
pub(crate) const CARD_PAD_Y: f32 = 6.0;
/// 第一行文字的中心相对基线上移的距离（字号的倍数，取自调参）：把手与汉字、小写字母的
/// 视觉中心对齐。
const CENTER_RISE: f32 = 0.35;
/// 让位偏移逼近目标的时间常数（秒，取自调参）：约 0.11 秒（三倍时间常数）走完 95% 的距离，
/// 插入位置连续变化时兄弟块平滑跟随，不逐帧跳动。
const SHIFT_TIME: f32 = 0.038;
/// 让位偏移与目标之差小于该值时立即到位（逻辑像素）：在 2 倍缩放的屏上不到半个物理像素。
const SHIFT_SETTLED: f32 = 0.25;

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

/// 预览中的拖动状态：指针下的单元、进行中的拖动、左侧留白的宽度和放下后的过渡。
#[derive(Default)]
pub(crate) struct DragState {
    /// 指针下的可拖动单元，用于判断悬停的单元是否改变；改变时重绘。
    pub(crate) hover: Option<Element>,
    /// 正在拖动的单元。
    pub(crate) session: Option<BlockDrag>,
    /// 顶层块放下后等待按新解析结果滑到新位置时，放下所得的文档版本；文档此后再修改时
    /// 不再套用。
    pub(crate) slide: Option<u64>,
    /// 列表项放下后的逐项滑动。
    pub(crate) item_motion: Option<ItemMotion>,
    /// 松手时各块当前的视觉偏移，重排过渡从这些位置继续，不跳回原位。
    pub(crate) release: Option<ReleasedShift>,
    /// 顶层列表的拆分结果，按 (解析结果, 文档版本) 缓存，避免鼠标移动时反复复制列表原文。
    list_cache: RefCell<ListCache>,
    /// 最近一次列表项几何，避免悬停和开始拖动时重复计算。
    shape_cache: RefCell<Option<ShapeCache>>,
    /// 松手后、新解析到达前仍然显示在松手位置的浮动卡片。
    pub(crate) drop_card: Option<DropCard>,
}

/// 松手后等待新解析期间保留的浮动卡片。
pub(crate) struct DropCard {
    /// 单元所在的顶层块序号。
    pub(crate) block: usize,
    /// 拖动的单元。
    pub(crate) element: Element,
    /// 卡片内容的左边缘、宽度和高度（逻辑像素），同 [`BlockDrag`] 的同名字段。
    pub(crate) left: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// 单元上边缘相对顶层块上边缘的偏移（逻辑像素）。
    pub(crate) offset: f32,
    /// 卡片上边缘（窗口坐标）。
    pub(crate) top: f32,
    /// 松手时兄弟块或列表项的让位偏移，等待新解析期间保持不变。
    pub(crate) shift: Vec<f32>,
    /// 改写原文之后的文档版本号。
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

    /// 返回 `block` 的列表项几何；排版条件、块和列表拆分均未改变时复用上一次的结果。
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

/// 拖动中兄弟块或列表项的让位偏移（逻辑像素），按指数平滑逼近目标。
pub(crate) struct DragShift {
    /// 各块或各项当前的偏移。
    current: Vec<f32>,
    /// 各块或各项的目标偏移；与 `current` 等长，逐帧复用。
    target: Vec<f32>,
    /// 上一次更新的时刻。
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

    /// 把 `count` 个偏移推向 `goal` 给出的目标，`animate` 为假时立即到位；返回是否仍在移动。
    fn update(&mut self, count: usize, goal: impl Fn(usize) -> f32, animate: bool) -> bool {
        self.current.resize(count, 0.0);
        self.target.clear();
        self.target.extend((0..count).map(goal));
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.at).as_secs_f32();
        self.at = now;
        let alpha = match animate {
            true => 1.0 - (-dt / SHIFT_TIME).exp(),
            false => 1.0,
        };
        let mut moving = false;
        for (current, &target) in self.current.iter_mut().zip(&self.target) {
            *current += (target - *current) * alpha;
            if (target - *current).abs() < SHIFT_SETTLED {
                *current = target;
            } else {
                moving = true;
            }
        }
        moving
    }

    /// 返回第 `index` 块或第 `index` 项当前的偏移（逻辑像素）。
    pub(crate) fn at(&self, index: usize) -> f32 {
        self.current.get(index).copied().unwrap_or(0.0)
    }

    fn moving(&self) -> bool {
        self.current
            .iter()
            .zip(&self.target)
            .any(|(current, target)| (target - current).abs() >= SHIFT_SETTLED)
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
    /// 单元在内容坐标中的中心：指针在中心上方时按向上拖动处理，在下方时按向下拖动处理。
    pub(crate) center: f32,
    /// 按下的单元的高度、宽度和左边缘（逻辑像素）。
    pub(crate) height: f32,
    pub(crate) width: f32,
    pub(crate) left: f32,
    /// 单元上边缘相对顶层块上边缘的偏移（逻辑像素）；列表项的浮层按该偏移平移块的内容。
    pub(crate) offset: f32,
    /// 抬起阴影、浮层出现的过渡；动态效果关闭时为 `None`。
    pub(crate) lift: Option<Clock>,
    /// 插入位置指示线滑动到新位置的过渡；尚未移动过时为 `None`。
    pub(crate) indicator: Option<Indicator>,
    /// 兄弟块/列表项的让位偏移。
    pub(crate) shift: DragShift,
    /// 按下时文档的标识与版本号：两者任一改变时取消拖动，不改写原文。
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

    /// 返回指针在纵坐标 `y` 时浮动卡片的上边缘（窗口坐标）：随抬起上移 [`CARD_LIFT`]，
    /// 连同留白限制在 `area` 内。
    pub(crate) fn card_top(&self, area: Rect, y: f32) -> f32 {
        let lowest = (area.bottom() - self.height - CARD_PAD_Y).max(area.y + CARD_PAD_Y);
        (y - self.grab_y - CARD_LIFT * self.lifted()).clamp(area.y + CARD_PAD_Y, lowest)
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
    /// 被拖动的块在旧排版和新排版中的序号；内容相同的块按此纠正对应关系。
    pub(crate) dragged: Option<(usize, usize)>,
    /// 松开时文档的版本号；之后的编辑不再套用这些偏移。
    pub(crate) revision: u64,
}

/// 列表项放下后的滑动：旧顺序各项的上边缘、新顺序到旧顺序的映射和时钟。
pub(crate) struct ItemMotion {
    /// 列表所在的顶层块序号。
    pub(crate) block: usize,
    /// 改写原文之后的文档版本号；文档再修改时放弃滑动。
    revision: u64,
    /// 松手时各项按旧顺序的上边缘（内容坐标，含让位偏移）。
    old_tops: Vec<f32>,
    /// 新顺序第 i 项在旧顺序中的序号。
    order: Vec<usize>,
    /// 新解析结果到达后才启动；等待期间不移动。
    clock: Option<Clock>,
    /// 创建时刻；等待解析超过一秒时放弃，不再持续请求动画帧。
    created: Instant,
}

impl ItemMotion {
    /// 过渡是否已经结束；尚未开始（等待新解析结果）时不算结束。
    pub(crate) fn finished(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|clock| clock.progress() >= 1.0)
    }
}

/// 一帧中各列表项的纵向偏移，以及把绘制项归到某一项所用的形状。
pub(crate) struct ItemSlide {
    /// 偏移的来源。
    offsets: Offsets,
    /// 各项的形状；项数与偏移的个数一致，创建时已核对。
    shapes: Rc<Vec<ItemShape>>,
}

/// 列表项纵向偏移的来源：绘制时按项读取，不逐帧复制。
enum Offsets {
    /// 拖动中的让位偏移（[`BlockDrag::shift`]）。
    Dragging,
    /// 松手后等待新解析期间保持的偏移（[`DropCard::shift`]）。
    Dropped,
    /// 放下后的滑动（[`ItemMotion`]），值为尚未走完的比例，从 1 降到 0。
    Sliding(f32),
}

impl ItemSlide {
    /// 返回第 `item` 项这一帧的纵向偏移（逻辑像素）。
    fn at(&self, md: &MarkdownPreview, item: usize) -> f32 {
        let offset = match self.offsets {
            Offsets::Dragging => md.drag.session.as_ref().map(|drag| drag.shift.at(item)),
            Offsets::Dropped => {
                (md.drag.drop_card.as_ref()).and_then(|card| card.shift.get(item).copied())
            }
            Offsets::Sliding(left) => md.drag.item_motion.as_ref().and_then(|motion| {
                let old = motion.old_tops.get(*motion.order.get(item)?)?;
                Some((old - self.shapes.get(item)?.top) * left)
            }),
        };
        offset.unwrap_or(0.0)
    }

    /// 各列表项这一帧的纵向偏移（测试用）。
    #[cfg(test)]
    pub(crate) fn offsets(&self, md: &MarkdownPreview) -> Vec<f32> {
        (0..self.shapes.len()).map(|i| self.at(md, i)).collect()
    }

    /// 返回内容坐标 `y` 所在列表项这一帧的纵向偏移（逻辑像素）。
    pub(crate) fn offset(&self, md: &MarkdownPreview, y: f32) -> f32 {
        let i = self.shapes.partition_point(|shape| shape.top <= y);
        self.at(md, i.saturating_sub(1))
    }
}

/// 一个拖动单元在屏幕上的几何（逻辑像素）。
struct ElementGeometry {
    /// 上边缘（窗口坐标，含过渡动画）。
    top: f32,
    /// 第一行文字中心（窗口坐标）。
    center: f32,
    /// 高度。
    height: f32,
    /// 浮层宽度。
    width: f32,
    /// 相对所在块内容顶部的纵向偏移。
    offset: f32,
    /// 内容坐标中的起点。
    boundary: f32,
}

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
        // 左侧留白中的把手、标题折叠箭头和行号同样算在内；右侧只到正文一栏的右边缘。
        let left = handle_left(laid.left, app.hits.markdown_numbers).max(area.x);
        if x < left || x > laid.left + laid.width() {
            return None;
        }
        let top = area.y + laid.pad - crate::layout::drawn_scroll(self.scroll());
        let y = y - top;
        // 按块的上边缘划分整个纵向范围，块之间的间距归上一块：指针从一段移向下一段时把手
        // 不消失。
        let block = laid.tops.partition_point(|&top| top <= y).saturating_sub(1);
        laid.blocks.get(block)?;
        // 顶层列表按项划分：每项一个把手，拖动时只移动这一项。
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

    /// 返回开始拖动时单元在屏幕上的几何：上边缘、第一行中心、高度、浮层宽度、相对块顶的
    /// 偏移和内容坐标中的起点。
    ///
    /// 浮层宽度只取内容本身到达的最右处：代码块、引用的底色和表格的外框铺满一栏，按外框定宽
    /// 时标题的浮层与整栏同宽，插入位置指示线被完全盖住。
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
                    width: entry.block.content_right().clamp(MIN_CARD, laid.width()),
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

    /// 返回单元第一行文字的中心（窗口坐标）。
    fn handle_center(&self, app: &App, element: Element) -> Option<f32> {
        let laid = self.layout.as_ref()?;
        let block_top = self.block_top(element.block())?;
        match element {
            Element::Block(block) => {
                Some(first_line_center(&laid.blocks.get(block)?.block, block_top))
            }
            Element::Item { block, item } => {
                let items = self.drag.list_items(app, block)?;
                let shapes = self.drag.item_shapes(laid, block, &items)?;
                Some(block_top + shapes.get(item)?.center - laid.tops[block])
            }
        }
    }

    /// 返回把手在当前帧的矩形（窗口坐标）；单元不可拖动时为 `None`。
    pub(crate) fn handle_rect(&self, app: &App, element: Element) -> Option<Rect> {
        if !element_movable(app, element) {
            return None;
        }
        let center = self.handle_center(app, element)?;
        let x = handle_left(self.layout.as_ref()?.left, app.hits.markdown_numbers).max(self.area.x);
        Some(Rect::new(
            x,
            center - HANDLE_HEIGHT / 2.0,
            HANDLE,
            HANDLE_HEIGHT,
        ))
    }

    /// 返回列表项放下后的滑动；动画已结束、块不匹配、文档已修改或新解析尚未到达时为 `None`。
    pub(crate) fn item_slide(&self, app: &App, block: usize) -> Option<ItemSlide> {
        let motion = self.drag.item_motion.as_ref()?;
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
        Some(ItemSlide {
            offsets: Offsets::Sliding(1.0 - clock.progress()),
            shapes,
        })
    }

    /// 新排版到达或文档再次修改后，移除等待中的浮动卡片。
    pub(crate) fn clear_drop_card(&mut self, revision: u64) {
        let done =
            self.drag.drop_card.as_ref().is_some_and(|card| {
                card.revision != revision || self.checked == Some(card.revision)
            });
        if done {
            self.drag.drop_card = None;
        }
    }

    /// 返回拖动中或松手后等待新解析的单元：拖动中取 [`DragState::session`]，否则取
    /// [`DragState::drop_card`]。
    pub(crate) fn dragged_element(&self) -> Option<Element> {
        match (&self.drag.session, &self.drag.drop_card) {
            (Some(drag), _) => Some(drag.element),
            (None, Some(card)) => Some(card.element),
            (None, None) => None,
        }
    }

    /// 返回拖动中的列表项让位偏移：（列表所在的块，各项的偏移）；拖动的不是列表项时为 `None`。
    pub(crate) fn drag_item_slide(&self, app: &App) -> Option<(usize, ItemSlide)> {
        let Element::Item { block, .. } = self.dragged_element()? else {
            return None;
        };
        let (offsets, count) = match (&self.drag.session, &self.drag.drop_card) {
            (Some(drag), _) => (Offsets::Dragging, drag.shift.current.len()),
            (None, Some(card)) => (Offsets::Dropped, card.shift.len()),
            (None, None) => return None,
        };
        let laid = self.layout.as_ref()?;
        let items = self.drag.list_items(app, block)?;
        let shapes = self.drag.item_shapes(laid, block, &items)?;
        (shapes.len() == count).then_some((block, ItemSlide { offsets, shapes }))
    }

    /// 返回拖动中的列表项：（所在的块，原文字节范围（相对顶层块），内容坐标中的上下边缘）。
    ///
    /// 绘制时跳过该项的原文，原位置由让位的兄弟项占据；整块遮罩会把滑进原位置的兄弟项一并盖住。
    pub(crate) fn dragged_item(
        &self,
        app: &App,
    ) -> Option<(usize, std::ops::Range<usize>, f32, f32)> {
        let Element::Item { block, item } = self.dragged_element()? else {
            return None;
        };
        let laid = self.layout.as_ref()?;
        let items = self.drag.list_items(app, block)?;
        let range = items.ranges.get(item)?.clone();
        let shapes = self.drag.item_shapes(laid, block, &items)?;
        let shape = shapes.get(item)?;
        Some((block, range, shape.top, shape.bottom))
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
    // 放下后原文已经改写，新的解析和排版可能尚未到达，旧范围可能落在新文字的 UTF-8 字符中间：
    // 排版不是按当前解析结果得到时不拆分列表。
    if laid.source != md.parse_serial {
        return None;
    }
    let range = laid.ranges.get(block)?;
    let rope = app.editor.active()?.text().rope()?;
    let start_byte = range.start.min(rope.len_bytes());
    let end_byte = range.end.min(rope.len_bytes()).max(start_byte);
    // 范围不在字符边界上时按不是列表处理，绘制时不 panic。
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

/// 浮动卡片内容的最小宽度（逻辑像素，取自调参）：只有一两个字的块仍显出卡片的形状。
const MIN_CARD: f32 = 64.0;

/// 返回上边缘在 `top` 处的块第一行文字的中心（窗口坐标，逻辑像素）。
fn first_line_center(block: &winc_typeset::LaidBlock, top: f32) -> f32 {
    use winc_typeset::Item;
    for item in &block.items {
        if let Item::Text { baseline, font, .. } = item {
            return top + baseline - font.size * CENTER_RISE;
        }
    }
    top + (block.height / 2.0).min(HANDLE / 2.0)
}

/// 第 `block` 块在排版和解析结果中是否可以作为一个整体移动。
///
/// 看不见的块和脚注不可拖动（见 [`drop::visible_block`]）；收起的标题不可拖动，否则只移动
/// 标题，收起的一节留在原处。
fn block_movable(laid: &Laid, parsed: &Markdown, len: usize, block: usize) -> bool {
    drop::visible_block(laid, parsed, len, block) && !drop::folded_heading(laid, block)
}

/// 单元是否可以拖动：只看预览、文档可以编辑，并且解析结果与当前文字一致；文首的
/// front matter 不可拖动。
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
    if !block_movable(laid, parsed, len, block) || (block == 0 && drop::front_matter(app)) {
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

/// 预览中是否显示拖动把手：设置已打开，预览显示在编辑区（只看预览或分栏右栏），文档可以编辑。
///
/// 解析结果是否与当前文字一致由 [`drag_ready`] 判断，左侧留白只按本函数保留。docx、EPUB 的
/// 预览和阅读标签由转换得到，块与原文位置不对应；LaTeX 预览编译成 PDF，不经过此处。
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
/// 解析或排版进行中时把手暂时不响应，左侧留白仍按 [`shows_handles`] 保留，正文不左右移动。
fn drag_ready(app: &App) -> bool {
    let Some(doc) = app.editor.active() else {
        return false;
    };
    let md = &app.preview.markdown;
    shows_handles(app) && md.checked == Some(doc.revision) && md.pending.is_none()
}

/// 返回正文左侧为把手、折叠箭头和行号栏预留的宽度（逻辑像素）。
///
/// 从左到右依次是把手、折叠箭头、行号栏、正文；没有行号时行号栏的宽度为 0，箭头与正文之间
/// 留 8 像素。`numbers` 是行号栏（含与正文的间距）的宽度，绘制行号时由 `number_room` 量出。
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
    /// 更新指针下的可拖动单元；单元改变时重绘，以显示或隐藏把手。
    pub(in crate::app) fn track_markdown_drag_hover(&mut self, x: f32, y: f32) {
        let element = self.preview.markdown.element_at(self, x, y);
        if element != self.preview.markdown.drag.hover {
            self.preview.markdown.drag.hover = element;
            self.request_redraw();
        }
    }

    /// 返回窗口坐标 `(x, y)` 处的把手所在的单元。
    pub(in crate::app) fn markdown_handle_at(&self, x: f32, y: f32) -> Option<Element> {
        let md = &self.preview.markdown;
        // 按当前位置重新查找单元：解析结果和块序号可能在上一次指针移动之后改变。
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
            .drag
            .item_motion
            .as_ref()
            .is_some_and(|motion| {
                revision != Some(motion.revision)
                    || (motion.clock.is_none() && motion.created.elapsed() > Duration::from_secs(1))
            });
        if stale {
            self.preview.markdown.drag.item_motion = None;
            return;
        }
        let ready = self
            .preview
            .markdown
            .drag
            .item_motion
            .as_ref()
            .is_some_and(|motion| {
                motion.clock.is_none() && self.preview.markdown.checked == Some(motion.revision)
            });
        if !ready {
            return;
        }
        let Some(clock) = self.drag_clock(crate::app::animation::DROP_SLIDE) else {
            self.preview.markdown.drag.item_motion = None;
            self.preview.markdown.drag.release = None;
            return;
        };
        if let Some(motion) = self.preview.markdown.drag.item_motion.as_mut() {
            motion.clock = Some(clock);
        }
    }

    /// 更新拖动中的让位偏移：原位置与放置位置之间的兄弟块或列表项向空出的一侧让开一格。
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
        let slot = match target {
            Boundary::Block(slot) | Boundary::Item(slot) => slot,
        };
        // 向下拖时原位置之后、放置位置之前的单元上移一格，向上拖时反之。
        let goal = |i: usize| {
            if (unit + 1..slot).contains(&i) {
                -step
            } else if (slot..unit).contains(&i) {
                step
            } else {
                0.0
            }
        };
        let animate = self.animations_on();
        let Some(session) = self.preview.markdown.drag.session.as_mut() else {
            return;
        };
        if session.shift.update(count, goal, animate) {
            self.request_redraw();
        }
    }

    /// 拖动中的文档或其版本与按下时不同（从磁盘重新读入、在其他窗格中编辑、撤销），或指针
    /// 已不在拖动块的状态时，取消拖动：原来的块序号和让位偏移不再对应当前的内容。
    pub(crate) fn keep_markdown_drag_current(&mut self) {
        let Some(drag) = self.preview.markdown.drag.session.as_ref() else {
            return;
        };
        let current = self
            .editor
            .active()
            .is_some_and(|doc| doc.id.0 == drag.doc && doc.revision == drag.revision);
        if !current || self.pointer.drag != Drag::MarkdownBlock {
            self.cancel_markdown_block_drag();
        }
    }

    /// 在把手处按下：开始拖动，不改动原文。
    ///
    /// 文档在按下之后、松开之前被修改过时，松开时不再改写原文，避免把旧的位置用到新文字上。
    pub(in crate::app) fn start_markdown_drag(&mut self, element: Element, y: f32) {
        if !element_movable(self, element) {
            return;
        }
        // 新的拖动从当前画面开始：清除上一次放下后的滑动、等待解析的浮动卡片和滑入标记。
        self.preview.markdown.motion = None;
        self.preview.markdown.drag.item_motion = None;
        self.preview.markdown.drag.release = None;
        self.preview.markdown.drag.drop_card = None;
        self.preview.markdown.drag.slide = None;
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

    /// 拖动中的指针移动：按指针的纵坐标更新放置位置，靠近上下边缘时自动滚动。
    pub(in crate::app) fn drag_markdown_block(&mut self, y: f32) {
        self.keep_markdown_drag_current();
        if self.preview.markdown.drag.session.is_none() {
            return;
        }
        // 浮层随指针移动：指针在同一插入位置内移动时同样重绘。
        self.request_redraw();
        self.scroll_while_dragging_block(y);
        let area = self.preview.markdown.area;
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
        // 指针在单元原中心上方时按向上拖动处理，越过的块立即让位；在下方时按向下拖动处理。
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

    /// 指针在预览上下边缘以内 [`EDGE`] 时按深入的距离滚动，滚动量按经过的时间计算。
    ///
    /// 指针停在边缘不动时由 [`App::continue_drag_scroll`] 逐帧调用，滚动速度不受指针事件的
    /// 频率影响。
    fn scroll_while_dragging_block(&mut self, y: f32) {
        let area = self.preview.markdown.area;
        let depth = if y < area.y + EDGE {
            y - (area.y + EDGE)
        } else if y > area.bottom() - EDGE {
            y - (area.bottom() - EDGE)
        } else {
            0.0
        };
        if depth == 0.0 {
            self.pointer.drag_scrolled_at = None;
            return;
        }
        let line = self.fonts.line_height.max(1.0);
        let step = self.drag_scroll_lines(depth.abs() / line) * line;
        let scroll = self.preview.markdown.scroll();
        self.scroll_markdown_to(scroll + step.copysign(depth));
    }

    /// 拖动块时是否需要逐帧自动滚动：指针在预览上下边缘以内，且该方向仍可滚动。
    pub(in crate::app) fn markdown_drag_autoscrolling(&self) -> bool {
        let md = &self.preview.markdown;
        let (area, y) = (md.area, self.pointer.mouse.1);
        md.drag.session.is_some()
            && ((y < area.y + EDGE && md.scroll() > 0.0)
                || (y > area.bottom() - EDGE && md.scroll() < md.max_scroll()))
    }

    /// 放弃正在进行的块拖动：不改动原文，恢复指针状态。
    pub(in crate::app) fn cancel_markdown_block_drag(&mut self) {
        if self.preview.markdown.drag.session.take().is_some() {
            self.preview.markdown.drag.hover = None;
            self.pointer.drag_scrolled_at = None;
            self.request_redraw();
        }
        if self.pointer.drag == Drag::MarkdownBlock {
            self.pointer.drag = Drag::None;
            self.refresh_pointer_hover();
            self.update_cursor_icon();
        }
    }

    /// 把松开拖动时各块的视觉偏移叠加到放下后的重排过渡 `moves` 上，各块从松手时的位置滑动。
    ///
    /// `before` 和 `laid` 是重排前后的排版，`scroll` 是滚动位置。内容相同的块按先后对应时，
    /// 被拖动的块可能与同内容的块对调，按 [`ReleasedShift::dragged`] 纠正。
    pub(crate) fn carry_drag_release(
        &mut self,
        before: &Before,
        laid: &Laid,
        scroll: f32,
        moves: &mut Moves,
    ) {
        let Some(release) = self.preview.markdown.drag.release.take() else {
            return;
        };
        if self
            .editor
            .active()
            .is_none_or(|doc| doc.revision != release.revision)
        {
            return;
        }
        // `matched` 的值是旧排版的块序号，下标是新排版的块序号，两者分别在范围内。
        let from = |old: usize, new: usize| before.blocks[old].top - (laid.tops[new] - scroll);
        if let Some((old, new)) = release.dragged
            && let Some(current) = moves.matched.iter().position(|m| *m == Some(old))
            && current != new
            && new < moves.matched.len()
        {
            let other = moves.matched[new];
            moves.matched.swap(current, new);
            moves.from[new] = from(old, new);
            if let Some(other) = other {
                moves.from[current] = from(other, current);
            }
        }
        for (i, old) in moves.matched.iter().enumerate() {
            if let Some(offset) = old.and_then(|old| release.offsets.get(old)) {
                moves.from[i] += offset;
            }
        }
    }

    /// 松开鼠标：把块移到当前的放置位置，改写一次原文。返回是否改动了原文。
    pub(in crate::app) fn finish_markdown_block_drag(&mut self) -> bool {
        let Some(drag) = self.preview.markdown.drag.session.take() else {
            return false;
        };
        self.preview.markdown.drag.hover = None;
        self.request_redraw();
        // 拖到预览外面（侧栏、顶栏）再松开：不移动。上下各放宽一个自动滚动的距离，指针略超出
        // 边缘时仍按最近的边界放下。
        let area = self.preview.markdown.area;
        let (mx, my) = self.mouse();
        if mx < area.x || mx > area.right() || !(area.y - EDGE..=area.bottom() + EDGE).contains(&my)
        {
            return false;
        }
        let Some(landing) = self.plan_drop(&drag) else {
            return false;
        };
        if !self.commit_text_move(&landing.moved) {
            return false;
        }
        self.land_drop(&drag, landing);
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
    // 向上拖时指针进入某一项的下边缘即插到该项之前，向下拖时进入上边缘即插到该项之后：
    // 指针进入相邻项时该项立即让位，不必越过该项的中线。
    let edge = |shape: &ItemShape| if up { shape.bottom } else { shape.top };
    let slot = (shapes.iter())
        .position(|shape| y < edge(shape))
        .unwrap_or(shapes.len());
    let end = layout.tops[block] + layout.blocks[block].block.height;
    let boundary = shapes.get(slot).map_or(end, |shape| shape.top);
    Some((slot, boundary))
}

/// 目标边界是否可以放置并会改变顺序。
///
/// 原位和紧邻原位不改变顺序；文首 front matter 之前、收起的一节之中（含该节与下一个标题之间）
/// 不可放置。
fn boundary_effective(app: &App, parsed: &Markdown, element: Element, target: Boundary) -> bool {
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return false;
    };
    match (element, target) {
        (Element::Block(block), Boundary::Block(target)) => {
            let (count, len) = (laid.blocks.len(), parsed.len);
            let placeable = target == count || drop::visible_block(laid, parsed, len, target);
            let next = (block + 1..count)
                .find(|&i| drop::visible_block(laid, parsed, len, i))
                .unwrap_or(count);
            placeable
                && target != block
                && target != next
                && !drop::inside_fold(laid, parsed, len, target)
                && !(target == 0 && drop::front_matter(app))
        }
        (Element::Item { item, .. }, Boundary::Item(slot)) => slot != item && slot != item + 1,
        _ => false,
    }
}

/// 返回距离内容坐标 `y` 最近的块边界序号：插到第几块之前，等于块数时表示插到最后。
///
/// 看不见的块和脚注不算边界（见 [`drop::visible_block`]）：放在这些块之前的位置会被误认为
/// 移到了别处。收起的标题之前可以放置；front matter 之前、收起的一节之中的边界仍会返回，由
/// [`boundary_effective`] 判为不可放置，不显示插入线。
fn boundary_near(laid: &Laid, parsed: &Markdown, y: f32, up: bool) -> usize {
    // 向上拖时，指针进入某块的下边界即插到该块之前；向下拖时进入上边界即插到该块之后。
    for i in 0..laid.blocks.len() {
        if !drop::visible_block(laid, parsed, parsed.len, i) {
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

mod drop;
mod rewrite;
mod shapes;

use shapes::{ItemShape, item_shapes};
