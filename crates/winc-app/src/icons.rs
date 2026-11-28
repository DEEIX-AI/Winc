//! 界面图标：外形来自 CPUI 的图标集（`cpui-icons`），在运行时描边绘制，不依赖图片资源。
//!
//! 所有图标共用一套线条风格，与顶栏的分段控件、文件树的折叠箭头相同：圆头描边，笔宽
//! `STROKE`，小图标里的记号用 `GLYPH_STROKE`。加载、进度这类动画与勾选框等控件仍由各函数
//! 自行绘制。
//!
//! 图标尺寸以「宽×高」标注，单位为逻辑像素。多数图标以中心点定位；文件、目录、新建文件与文档
//! 图标以左上角定位，具体见各函数的参数说明。

mod files;
mod search;
mod settings;
mod symbols;
mod writing;

pub use files::*;
pub use search::*;
pub use settings::*;
pub use symbols::*;
pub use writing::*;

use std::f32::consts::{FRAC_PI_2, PI};

use cpui_render::{Color, Frame, Rect, Stroke};

/// 线条图标的笔宽（逻辑像素）。
///
/// 顶栏、目录、新建文件、工具栏图标与文件内的记号共用该笔宽，并排放在同一行时笔画粗细一致。
/// 取值为调参结果：笔宽在 1.25 逻辑像素时，圆头描边在 1 倍与 2 倍像素密度下都不出现断点。
const STROKE: f32 = 1.25;
/// 文件图标纸面上记号的笔宽（逻辑像素），等于 `STROKE`。
const GLYPH_STROKE: f32 = STROKE;
/// 文件图标的记号相对设计稿放大的倍数（无单位）。
///
/// 设计稿的格子为 ±3.5 逻辑像素；按该倍数放大后记号约 11 逻辑像素见方，与目录图标的外形尺寸
/// 相同。取值为调参结果：放大到该倍数时记号在图标纸面内留出约 1 逻辑像素的边距。
const MARK_SCALE: f32 = 1.55;
/// 箭头（折叠展开、路径分隔、下拉）的笔宽（逻辑像素）。
///
/// 箭头只有两笔，按 `STROKE` 绘制时视觉重量高于同一行的文字，因此单独取更细的笔宽；取值为
/// 调参结果，作用范围限于上述三类箭头。
const CHEVRON_STROKE: f32 = 1.2;
/// 勾选框内的勾与差异视图「»」按钮的笔宽（逻辑像素）。
///
/// 两者在差异视图里并排显示，取同一个值以保证笔画粗细一致；取值为调参结果。
const MARK_STROKE: f32 = 1.5;
/// 文件图标的宽度（逻辑像素）。
pub const FILE_WIDTH: f32 = 12.0;
/// 目录图标的宽度（逻辑像素），也是文件树中目录图标与文件图标所占列的列宽。
///
/// 目录图标与文件图标同在一列时，列宽取该值，文件图标在列内居中向右偏移
/// `(FOLDER_WIDTH - FILE_WIDTH) / 2`，两种图标的中心对齐，名称从同一位置起始。取值由
/// 目录图标的设计稿宽度决定。
pub const FOLDER_WIDTH: f32 = 15.0;
/// 侧栏开关图标（`sidebar`）的宽度（逻辑像素）。取值为调参结果：宽度为偶数，取整后左右两侧
/// 的留白对称。
pub const SIDEBAR_WIDTH: f32 = 16.0;

/// 箭头 `V` 形折线的开口朝向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 朝上。
    Up,
    /// 朝下。
    Down,
    /// 朝右。
    Right,
    /// 朝左。
    Left,
}

/// 按给定朝向绘制 `V` 形折线箭头。
///
/// `size` 是箭头两端尖点的连线长度（逻辑像素）：朝上或朝下时为宽度，朝右时为高度。
pub fn chevron(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    direction: Direction,
    color: Color,
    clip: Rect,
) {
    let angle = match direction {
        Direction::Up => -FRAC_PI_2,
        Direction::Down => FRAC_PI_2,
        Direction::Right => 0.0,
        Direction::Left => PI,
    };
    chevron_turned(frame, center, size, angle, color, clip);
}

/// 绘制自朝右状态顺时针转过 `angle` 弧度（`f32`，弧度制）的 `V` 形折线箭头。
///
/// 目录展开、收起时，箭头在朝右与朝下之间按动画进度转过中间的弧度。
pub fn chevron_turned(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    angle: f32,
    color: Color,
    clip: Rect,
) {
    let (icon, extent, width) = (Icon::ChevronRight, fit(size, 12.0), CHEVRON_STROKE);
    draw_turned(frame, icon, center, extent, angle, width, color, clip);
}

/// 绘制收起图标：朝向 `direction` 的箭头，箭头尖端外侧一条边线，表示面板收入的方向。
///
/// 单独的箭头是下拉菜单的图标，边线用于区分两者。
pub fn collapse(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    direction: Direction,
    color: Color,
    clip: Rect,
) {
    let icon = match direction {
        Direction::Up => Icon::CollapseUp,
        Direction::Down => Icon::CollapseDown,
        Direction::Right => Icon::CollapseRight,
        Direction::Left => Icon::CollapseLeft,
    };
    // 箭头与边线横跨 8 逻辑像素。
    let extent = fit(8.0, 12.0);
    draw(frame, icon, center, extent, CHEVRON_STROKE, color, clip);
}

/// 绘制弹出菜单按钮右端上下排列的一对小箭头，整体宽 6、高 8 逻辑像素。
pub fn up_down(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::ChevronUpDown, fit(8.0, 16.0));
    draw(frame, icon, center, extent, CHEVRON_STROKE, color, clip);
}

/// 绘制外链箭头，9×9：指向右上方的箭头，表示在浏览器中打开。
pub fn external(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::ArrowUpRight, fit(7.0, 12.0));
    draw(frame, icon, center, extent, CHEVRON_STROKE, color, clip);
}

/// 绘制提醒图标，12×11：三角形内一个感叹号。
pub fn warning(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Warning, 2.0 * RING);
    draw(frame, icon, center, extent, GLYPH_STROKE, color, clip);
}

/// 绘制信息图标，11×11：圆圈内一个「i」，与 [`warning`] 同一大小。
pub fn info(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Info, 2.0 * RING);
    draw(frame, icon, center, extent, GLYPH_STROKE, color, clip);
}

/// 绘制完成图标，11×11：圆圈内一个勾。
pub fn done(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Success, 2.0 * RING);
    draw(frame, icon, center, extent, GLYPH_STROKE, color, clip);
}

/// 绘制出错图标，11×11：圆圈内一个感叹号。
pub fn error(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Alert, 2.0 * RING);
    draw(frame, icon, center, extent, GLYPH_STROKE, color, clip);
}

/// 绘制关闭图标（×），`size` 是叉的边长。
pub fn close(frame: &mut Frame<'_>, center: (f32, f32), size: f32, color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Close, fit(size, 12.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制窗口的最小化图标：一条横线，`size` 是线长。
pub fn minimize(frame: &mut Frame<'_>, center: (f32, f32), size: f32, color: Color, clip: Rect) {
    minus(frame, center, size, color, clip);
}

/// 绘制窗口的最大化图标：一个方框，`size` 为边长；`restore` 为 `true` 时绘制还原图标，即前后
/// 错开的两个方框。
pub fn maximize(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    restore: bool,
    color: Color,
    clip: Rect,
) {
    let icon = if restore {
        Icon::WindowRestore
    } else {
        Icon::WindowMaximize
    };
    draw(frame, icon, center, fit(size, 16.0), STROKE, color, clip);
}

/// 绘制加号，`size` 是边长。
pub fn plus(frame: &mut Frame<'_>, center: (f32, f32), size: f32, color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Plus, fit(size, 14.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制减号，笔宽和长度与加号相同。
pub fn minus(frame: &mut Frame<'_>, center: (f32, f32), size: f32, color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Minus, fit(size, 14.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 至多 `N` 个笔画的定长缓冲，用于在栈上拼接一个图标的笔画。
///
/// 图标每帧都绘制，因此不为笔画在堆上分配内存。超出容量的笔画被丢弃；容量按各图标实际用到的
/// 笔画数取，超出容量只在调试构建中由断言检查。
struct Strokes<const N: usize> {
    items: [Stroke; N],
    len: usize,
}

impl<const N: usize> Strokes<N> {
    fn new() -> Self {
        Self {
            items: [Stroke::Line(0.0, 0.0, 0.0, 0.0); N],
            len: 0,
        }
    }

    fn push(&mut self, stroke: Stroke) {
        debug_assert!(self.len < N, "图标的笔画超过 {N} 个");
        if let Some(slot) = self.items.get_mut(self.len) {
            *slot = stroke;
            self.len += 1;
        }
    }

    fn extend(&mut self, strokes: impl IntoIterator<Item = Stroke>) {
        strokes.into_iter().for_each(|s| self.push(s));
    }

    fn as_slice(&self) -> &[Stroke] {
        &self.items[..self.len]
    }
}

/// 图标集里的图标。图标的外形来自 CPUI 的图标集（`cpui-icons`），各绘制函数只决定位置和大小。
pub use cpui_icons::IconName as Icon;

/// 图标集的图标占网格的比例：图标的外形约占 24 格中的 18 格，按外形尺寸换算网格尺寸时用。
const GRID_PER_EXTENT: f32 = cpui_icons::GRID / 18.0;

/// 返回外形约 `extent` 逻辑像素见方的图标所用的网格边长，供 [`shape`] 和 [`draw`] 使用。
fn grid(extent: f32) -> f32 {
    extent * GRID_PER_EXTENT
}

/// 返回图标 `icon` 的笔画：网格中心在 `(cx, cy)`，网格边长为 `size` 逻辑像素，绕中心顺时针转过
/// `angle` 弧度。
fn shape(icon: Icon, (cx, cy): (f32, f32), size: f32, angle: f32) -> Strokes<40> {
    let unit = size / cpui_icons::GRID;
    let half = cpui_icons::GRID / 2.0;
    let (sin, cos) = angle.sin_cos();
    // 屏幕坐标的 `y` 轴向下，该旋转在屏幕上表现为顺时针。
    let at = |(x, y): (f32, f32)| {
        let (dx, dy) = ((x - half) * unit, (y - half) * unit);
        (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
    };
    let mut out = Strokes::new();
    for part in &icon.shapes() {
        out.push(match *part {
            cpui_icons::Shape::Line { from, to } => {
                let ((x0, y0), (x1, y1)) = (at(from), at(to));
                Stroke::Line(x0, y0, x1, y1)
            }
            cpui_icons::Shape::Circle { center, radius } => {
                let (x, y) = at(center);
                Stroke::Circle(x, y, radius * unit)
            }
            cpui_icons::Shape::Arc {
                center,
                radius,
                start,
                end,
            } => {
                let (x, y) = at(center);
                let (start, end) = (start.to_radians() + angle, end.to_radians() + angle);
                Stroke::Arc(x, y, radius * unit, start, end)
            }
        });
    }
    out
}

/// 绘制图标 `icon`：外形约 `extent` 逻辑像素见方，中心在 `center`，笔宽 `width`。
fn draw(
    frame: &mut Frame<'_>,
    icon: Icon,
    center: (f32, f32),
    extent: f32,
    width: f32,
    color: Color,
    clip: Rect,
) {
    draw_turned(frame, icon, center, extent, 0.0, width, color, clip);
}

/// 绘制绕中心顺时针转过 `angle` 弧度的图标 `icon`，其余参数同 [`draw`]。
#[allow(clippy::too_many_arguments)]
fn draw_turned(
    frame: &mut Frame<'_>,
    icon: Icon,
    center: (f32, f32),
    extent: f32,
    angle: f32,
    width: f32,
    color: Color,
    clip: Rect,
) {
    let strokes = shape(icon, center, grid(extent), angle);
    frame.stroke(strokes.as_slice(), width, color, clip);
}

/// 返回图标在网格上占 `span` 格的一维画成 `size` 逻辑像素时，传给 [`draw`] 的外形尺寸。
fn fit(size: f32, span: f32) -> f32 {
    size * cpui_icons::GRID / (span * GRID_PER_EXTENT)
}

/// 把转过的圈数 `turn`（任意实数）换成 -π 至 π 之间的弧度。
///
/// 渲染圆弧时按起止角的 `atan2` 结果加或减一圈来确定角度范围，起点落在该区间才能保证终点不超过
/// 3π；终点超过 3π 时圆弧末段不绘制。
fn turn_angle(turn: f32) -> f32 {
    let angle = turn.rem_euclid(1.0) * 2.0 * PI;
    if angle >= PI { angle - 2.0 * PI } else { angle }
}

// 文件树工具栏的五个图标都是线条图标：笔宽相同，外形约 12×12，并排放置时粗细和大小一致。

/// 工具栏图标的笔宽（逻辑像素），等于 `STROKE`。
const RING_STROKE: f32 = STROKE;
/// 刷新与定位图标中圆的半径（逻辑像素）。
///
/// 工具栏其余图标的外形按该圆的直径（11 逻辑像素）确定大小，使并排显示的工具栏图标尺寸一致。
const RING: f32 = 5.5;

/// 绘制「新建文件」图标，10×12：右上角折起的纸，纸上一个加号。
pub fn add_file(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::FilePlus, 12.0);
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「打开文件」图标，10×12：右上角折起的纸，纸上一个指向右上方的箭头（同 [`external`]）。
pub fn open_file(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::FileOpen, 12.0);
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「新建目录」图标，13×11：左上角带标签的目录，中部一个加号。
pub fn add_folder(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::FolderPlus, 13.0);
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「刷新」图标，约 13×13：一个缺口在右上方的圆，末端为直角箭头。
pub fn refresh(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    refresh_turned(frame, center, 0.0, color, clip);
}

/// 回车图标的宽度（逻辑像素）。
pub const RETURN_KEY_WIDTH: f32 = 10.0;

/// 绘制回车键（↩）图标，约 10×9：从右上方向下，再折向左，末端为箭头。
pub fn return_key(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::ReturnKey, fit(RETURN_KEY_WIDTH, 14.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「发布」图标，约 10×11，和「刷新」图标等高：向上的箭头，下面是托盘。
pub fn upload(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::UploadLine, fit(2.0 * RING, 16.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「下载」图标，约 10×11，和「发布」图标上下对称：向下的箭头，下面是托盘。
pub fn download(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::DownloadLine, fit(2.0 * RING, 16.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制进度环，直径 11，和「刷新」图标的圆一样大：淡色 `track` 的整圆上，从 12 点钟起顺时针
/// 画出占整圈 `fraction`（0 到 1）的一段 `color`。
pub fn progress_ring(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    fraction: f32,
    color: Color,
    track: Color,
    clip: Rect,
) {
    frame.stroke(
        &[Stroke::Arc(cx, cy, RING, -PI, PI)],
        RING_STROKE,
        track,
        clip,
    );
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let start = -FRAC_PI_2;
        let arc = Stroke::Arc(cx, cy, RING, start, start + fraction * 2.0 * PI);
        frame.stroke(&[arc], RING_STROKE, color, clip);
    }
}

/// 返回加载图标当前旋转到的圈数，按进程启动以来的时间计算，各处的加载图标转速一致。
pub fn spin_turn() -> f32 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    start.elapsed().as_secs_f32() * crate::theme::SPIN_SPEED
}

/// 运行（实心三角形，宽 7、高 8）：行号栏这类窄处的运行按钮，描边的 [`play`] 在其中显得拥挤。
///
/// 图标集只有描边，实心的三角形在这里直接填充。三角形的重心在宽度的三分之一处，整体右移宽度的
/// 六分之一，看起来位于 `(cx, cy)` 的正中。
pub fn play_solid(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    let (w, h) = (7.0, 8.0);
    let left = cx - w / 2.0 + w / 6.0;
    let points = [(left, cy - h / 2.0), (left + w, cy), (left, cy + h / 2.0)];
    frame.polygon(&points, color, clip);
}

/// 绘制按 `turn`（圈数，可为任意实数）顺时针转动的四分之三圆弧，表示操作正在进行，半径为
/// `radius`（逻辑像素）。
pub fn spinner(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    radius: f32,
    turn: f32,
    color: Color,
    clip: Rect,
) {
    let angle = turn_angle(turn);
    let arc = Stroke::Arc(cx, cy, radius, angle, angle + 1.5 * PI);
    frame.stroke(&[arc], RING_STROKE, color, clip);
}

/// 绘制按 `turn`（圈数，可为任意实数）顺时针旋转后的「刷新」图标，表示操作正在进行。
pub fn refresh_turned(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    turn: f32,
    color: Color,
    clip: Rect,
) {
    // 圆在网格上直径 16 格，画成 `RING` 的圆。
    let (icon, extent) = (Icon::Refresh, fit(2.0 * RING, 16.0));
    let angle = turn_angle(turn);
    draw_turned(frame, icon, center, extent, angle, RING_STROKE, color, clip);
}

/// 绘制「同步」图标，约 12×12：同一个圆上相对的两段圆弧，末端各接一个直角箭头。
///
/// 用于与远程交换提交的操作；只重读本地状态的操作用 [`refresh`]。
pub fn sync(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    sync_turned(frame, center, 0.0, color, clip);
}

/// 绘制按 `turn`（圈数，可为任意实数）顺时针旋转后的「同步」图标，表示同步正在进行。
pub fn sync_turned(frame: &mut Frame<'_>, center: (f32, f32), turn: f32, color: Color, clip: Rect) {
    // 圆弧半径 5.4 逻辑像素，比「刷新」略小，留出两端箭头的位置。
    let (icon, extent) = (Icon::Sync, fit(10.8, 16.0));
    let angle = turn_angle(turn);
    draw_turned(frame, icon, center, extent, angle, RING_STROKE, color, clip);
}

/// 绘制「全部折叠」图标，11×12：圆角卡片内一道横线，卡片上方露出后面一张较窄卡片的上沿，表示
/// 卡片叠放。
pub fn collapse_all(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::CollapseAll, fit(12.0, 16.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「定位当前文件」图标，13×13：一个圆加四条由圆周向内伸出的刻度，中心留空。
pub fn locate(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Locate, fit(2.0 * RING, 16.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「以列表显示」图标，约 12×10：左侧三个圆点，右侧三道等长的横线，表示平铺的各行。
pub fn list_view(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::List, fit(11.0, 15.5));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「以树形显示」图标，约 12×10：顶格一道横线，其下两道缩进的横线，左侧的竖线和短横线
/// 把它们连到上一行，表示目录与其中的文件。
pub fn tree_view(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::ListTree, fit(12.0, 16.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「更多操作」图标，12×2：横排三个圆点，相邻圆心间距 4.5 逻辑像素，点径约 2 逻辑像素。
pub fn more(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    // 圆心在网格上相距 7 格；点按 2 逻辑像素描成实心点。
    draw(frame, Icon::More, center, fit(9.0, 14.0), 2.0, color, clip);
}

/// 绘制侧栏「文件」视图图标，约 13×14：线条风格（笔宽 `STROKE`，与顶栏其他图标一致），图形为前后叠
/// 放的两张纸。前面一张是完整的圆角框，内部两道较短的水平线表示文字行；后面一张错开在左上方，只露出
/// 上边和左边一段。
///
/// `scale` 是相对原大小的缩放倍数（视图切换中的小号图标传入小于 1 的值），笔宽不变。
pub fn files(frame: &mut Frame<'_>, center: (f32, f32), scale: f32, color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Files, 13.5 * scale);
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制放大镜图标，约 13×13：线条风格，一个圆框加右下的手柄，笔宽 `STROKE`。
///
/// 顶栏的查找按钮、空编辑区的「在文件中查找」与侧栏的「搜索」切换都使用该图标。`scale` 是相对
/// 原大小的缩放倍数，笔宽不变。
pub fn search(frame: &mut Frame<'_>, center: (f32, f32), scale: f32, color: Color, clip: Rect) {
    let extent = fit(11.5 * scale, 15.5);
    draw(frame, Icon::Search, center, extent, STROKE, color, clip);
}

/// 绘制 Git 图标，约 14×14：线条风格，一条主干、两个提交、一道四分之一圆弧，笔宽 `STROKE`，
/// 提交圆环内不铺底色，与 `files`、`search` 等顶栏图标风格一致。
///
/// `scale` 是相对原大小的缩放倍数，笔宽不变。
pub fn git(frame: &mut Frame<'_>, center: (f32, f32), scale: f32, color: Color, clip: Rect) {
    let (icon, extent) = (Icon::GitBranch, 12.6 * scale);
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制「回滚」图标，12×10：一道折返后指向左侧的箭头，表示撤回改动。
pub fn rollback(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Rollback, fit(10.0, 16.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制对勾图标，约 `size` 宽：短的一笔向右下，长的一笔向右上。
pub fn check(frame: &mut Frame<'_>, center: (f32, f32), size: f32, color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Check, fit(size, 14.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制「历史」图标，约 13×13：与 `refresh` 同样大小的圆，箭头方向相反（逆时针），圆内有时针
/// 与分针，表示查看提交记录。
///
/// 视觉重量与标题行的 `refresh`、`collapse_all` 等按钮一致。
pub fn history(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::History, fit(2.0 * RING, 17.0));
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制「生成」图标，约 13×13：左下一个大的四角星，右上一个小的；星的四条边向内凹。
pub fn sparkle(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Sparkle, center, 13.0, STROKE, color, clip);
}

/// 绘制「并排比较」图标，约 12×11：一个圆角框，中央一道竖线分成左右两栏，两栏内各一道短横。
pub fn compare(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Compare, center, 12.0, STROKE, color, clip);
}

/// 绘制小号分支图标，约 10×10：图形与 `git` 相同，用于状态栏与侧栏分支名旁的小号文字。
pub fn branch_small(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::GitBranch, center, 10.08, 1.25, color, clip);
}

/// 日志中引用徽章上的图标，表明引用的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefMark {
    /// 本地分支。
    Branch,
    /// 远程分支。
    Remote,
    /// 标签。
    Tag,
    /// 直接指向提交的 `HEAD`。
    Commit,
}

impl RefMark {
    /// 该种引用在图标集中的图标。
    fn icon(self) -> Icon {
        match self {
            RefMark::Branch => Icon::GitBranch,
            RefMark::Remote => Icon::Cloud,
            RefMark::Tag => Icon::Tag,
            RefMark::Commit => Icon::Commit,
        }
    }
}

/// 绘制引用徽章上的图标，约 9×9，笔宽 1 逻辑像素：与徽章中 12 号的文字相称。
///
/// 本地分支用分支图形，远程分支是一朵云，标签是尖端朝左的吊牌，直接指向提交的 `HEAD` 是提交图
/// 中的一个节点。
pub fn ref_mark(
    frame: &mut Frame<'_>,
    mark: RefMark,
    center: (f32, f32),
    color: Color,
    clip: Rect,
) {
    // 与 [`ref_icon`] 同一个图形，按徽章缩小到约 9 逻辑像素宽。
    let extent = match mark {
        RefMark::Branch => 9.0,
        RefMark::Commit => 11.0,
        RefMark::Remote => 9.2,
        RefMark::Tag => 9.1,
    };
    draw(frame, mark.icon(), center, extent, 1.0, color, clip);
}

/// 绘制小号「拉取」图标，约 10×10，笔宽 1.25：指向左下的箭头，提交从远程落到本地。
pub fn pull_small(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::ArrowDownLeft, fit(9.0, 12.0));
    draw(frame, icon, center, extent, 1.25, color, clip);
}

/// 绘制小号「推送」图标，约 10×10，笔宽 1.25：指向右上的箭头，与 [`pull_small`] 方向相反。
pub fn push_small(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::ArrowUpRight, fit(9.0, 12.0));
    draw(frame, icon, center, extent, 1.25, color, clip);
}

/// 绘制列表各行名字前面的引用图标，约 12×10，笔宽 1.25 逻辑像素：与 [`branch_small`] 同样粗细，
/// 用在小号文字旁。图形与 [`ref_mark`] 相同，按列表的行放大，横向不再压在 9 以内。
pub fn ref_icon(
    frame: &mut Frame<'_>,
    mark: RefMark,
    center: (f32, f32),
    color: Color,
    clip: Rect,
) {
    let extent = match mark {
        RefMark::Branch => 10.08,
        RefMark::Remote => 12.5,
        RefMark::Tag => 12.0,
        RefMark::Commit => 13.0,
    };
    draw(frame, mark.icon(), center, extent, 1.25, color, clip);
}

/// 绘制项目徽章右下角的工作树记号，约 9×9：一个底色为 `fill` 的圆，外圈露出底面 `ring`（与徽章
/// 底色隔开），内部为圈住的分支图形。
pub fn worktree_mark(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    (ring, fill, color): (Color, Color, Color),
    clip: Rect,
) {
    let disc = |r: f32| Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r);
    frame.surface_clipped(disc(6.0), 6.0, ring, clip);
    frame.rect_clipped(disc(4.75), 4.75, fill, clip);
    // 底色圆本身就是徽章的外框，里面只画分支。
    draw(frame, Icon::GitBranch, (cx, cy), 4.86, 1.0, color, clip);
}

/// 窗框类图标（侧栏开关、终端面板、分栏等）的外形宽度（逻辑像素）。
const FRAME: f32 = SIDEBAR_WIDTH - 1.5;

/// 返回窗框类图标取整后的中心：外框的边线落在固定的像素位置上，左右、上下的留白对称。
fn frame_center((cx, cy): (f32, f32)) -> (f32, f32) {
    let half = SIDEBAR_WIDTH / 2.0;
    ((cx - half).round() + half, (cy - 7.0).round() + 7.0)
}

/// 绘制侧栏开关图标，16×14：线条风格，一个圆角窗框，左侧一道短竖线表示侧栏。
///
/// `open` 为 `true`（侧栏显示）时竖线自上而下贯通，把窗框分成左右两栏；为 `false` 时竖线只剩中
/// 间一段。
///
/// 高度取 14 逻辑像素，与窗口按钮、`files`、`search` 图标的高度一致；该值为偶数，坐标取整后
/// 正好以 `cy` 为中心。
pub fn sidebar(frame: &mut Frame<'_>, center: (f32, f32), open: bool, color: Color, clip: Rect) {
    let icon = if open {
        Icon::Sidebar
    } else {
        Icon::SidebarCollapsed
    };
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 绘制「在对话框中打开」图标，约 13×11，与侧栏标题行上的「刷新」「全部折叠」等同高、同笔宽、
/// 同样只用线条：一个圆角窗口，正中浮着一张小卡片，表示浮在窗口上的对话框。与 [`panel`]（窗口
/// 底部隔出一条，表示放到底部面板）是一对。
pub fn float_window(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::FloatingWindow, 13.0);
    draw(frame, icon, center, extent, RING_STROKE, color, clip);
}

/// 绘制终端面板位置按钮的图标：与侧栏开关尺寸相同的圆角框；`right` 为 `true` 时在右侧隔出一
/// 栏，否则在下方隔出一条。
///
/// 调用方传入的是切换后的位置，图标表示按下后终端面板将放置的位置。
pub fn panel(frame: &mut Frame<'_>, center: (f32, f32), right: bool, color: Color, clip: Rect) {
    let icon = if right {
        Icon::PanelRight
    } else {
        Icon::PanelBottom
    };
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 绘制「移到编辑器标签」图标：与 [`panel`] 相同的圆角框和隔出的面板，一个箭头从面板的分隔线
/// 指向框中其余的部分，表示终端从面板移到编辑区。`right` 为 `true` 时面板在右侧，箭头朝左；
/// 否则面板在下方，箭头朝上。
pub fn to_editor(frame: &mut Frame<'_>, center: (f32, f32), right: bool, color: Color, clip: Rect) {
    let icon = if right {
        Icon::PanelRightToEditor
    } else {
        Icon::PanelBottomToEditor
    };
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 绘制「分栏」图标：与侧栏开关尺寸相同的圆角框，中央一道竖线分成左右两半（左侧为代码，右侧
/// 为预览）。
pub fn split(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let icon = Icon::SplitView;
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 绘制「单栏」图标：与 [`split`] 尺寸相同的圆角框，中间一道横线分成上下两半（上面是改之前，
/// 下面是改之后）。
pub fn single_column(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let icon = Icon::SingleColumn;
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 按缩放倍数 `k` 绘制 `split` 图标，用于出现与消失的动画。
///
/// 线宽不变，坐标不取整；`k` 不小于 1 时直接调用 `split` 绘制原尺寸图标。
pub fn split_scaled(frame: &mut Frame<'_>, center: (f32, f32), k: f32, color: Color, clip: Rect) {
    if k >= 1.0 {
        return split(frame, center, color, clip);
    }
    let (icon, extent) = (Icon::SplitView, FRAME * k);
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制终端图标：与侧栏开关尺寸相同的圆角框，内部为提示符 `>_`。
pub fn terminal(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let icon = Icon::Terminal;
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 绘制连接远端主机的图标：与终端图标同宽的上下两个圆角机箱，各有一道短线表示指示灯。
///
/// 用于状态栏，位于终端图标的左边，线宽和外框的宽度与它一致。
pub fn remote(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let icon = Icon::Server;
    let center = frame_center(center);
    draw(frame, icon, center, FRAME, STROKE, color, clip);
}

/// 绘制书签图标，7×11：竖条，下端中央有一个缺口。
///
/// 绘制在行号栏左侧，标记设置了书签的行。
pub fn bookmark(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    // 实心的标记，行号栏里要比描边的图标醒目，因此不用图标集。
    let (w, h, notch) = (7.0, 11.0, 3.0);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0);
    let tail = (w - 2.0) / 2.0;
    frame.rect_corners(
        Rect::new(x, y, w, h - notch),
        [1.5, 1.5, 0.0, 0.0],
        color,
        clip,
    );
    frame.rect_clipped(Rect::new(x, y + h - notch, tail, notch), 0.0, color, clip);
    frame.rect_clipped(
        Rect::new(x + w - tail, y + h - notch, tail, notch),
        0.0,
        color,
        clip,
    );
}

/// 绘制图钉图标，8×12：顶端的帽、向下张开的托和向下的针。
///
/// 用于钉住 Markdown 预览左侧的标题列表。
pub fn pin(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    // 实心的图钉，和行号栏的书签一致，因此不用图标集。
    let head = Rect::new(cx - 2.5, cy - 6.0, 5.0, 5.0);
    frame.rect_corners(head, [1.5, 1.5, 0.0, 0.0], color, clip);
    frame.rect_clipped(Rect::new(cx - 4.5, cy - 1.0, 9.0, 1.5), 0.75, color, clip);
    frame.rect_clipped(Rect::new(cx - 0.6, cy + 0.5, 1.2, 5.5), 0.6, color, clip);
}

/// 绘制「全屏」图标，约 12×12 逻辑像素：从中心分别指向右上角和左下角的两个箭头。
pub fn expand(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Expand, fit(10.0, 15.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制「退出全屏」图标，与 [`expand`] 成对：两个箭头从右上角、左下角指向中心。
pub fn shrink(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    let (icon, extent) = (Icon::Shrink, fit(10.0, 15.0));
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 绘制「复制」图标，约 12×12：两张错开叠放的圆角方块，前面一张完整，后面一张只露出左上角。
pub fn copy(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Copy, center, 11.5, STROKE, color, clip);
}
