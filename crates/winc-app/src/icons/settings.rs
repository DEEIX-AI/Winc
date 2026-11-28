//! 设置浮层左侧各页的图标（通用、外观、代码、Markdown、格式化、终端、模块、快捷键），以及锁、
//! 预览的眼睛、齿轮。

use super::*;

/// 绘制锁图标，9×12：线条风格，锁梁加圆角的锁身，锁身中间一道短竖线是锁孔。
pub fn lock(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::LockKeyhole,
        center,
        11.0,
        GLYPH_STROKE,
        color,
        clip,
    );
}

/// 绘制状态栏的锁图标，约 11×13；`open` 时锁梁右端不与锁身相接。
pub fn status_lock(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    open: bool,
    color: Color,
    clip: Rect,
) {
    let icon = if open {
        Icon::Unlock
    } else {
        Icon::LockKeyhole
    };
    // 锁的各部分是 [`lock`] 的 1.25 倍，笔宽不变。
    draw(
        frame,
        icon,
        (cx, cy - 0.5),
        11.0 * 1.25,
        GLYPH_STROKE,
        color,
        clip,
    );
}

/// 绘制「外观」图标，12×12：一个圆，一半实心，表示浅色和深色两套配色。
///
/// 用于设置的分类栏。
pub fn appearance(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Contrast,
        center,
        12.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「通用」图标，13×9：上下两道横线，各有一个圆形的滑块，表示可以调节的选项。
///
/// 用于设置的分类栏。
pub fn sliders(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Settings,
        center,
        13.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「编辑器」图标，14×11：一对尖括号，中间一道斜线，即「</>」。
///
/// 用于设置的分类栏。
pub fn code(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    code_scaled(frame, center, 1.0, color, clip);
}

/// 按比例 `k` 缩放绘制 [`code`] 图标，线宽不变；用于出现、消失的动画。
pub fn code_scaled(frame: &mut Frame<'_>, center: (f32, f32), k: f32, color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Code,
        center,
        14.0 * k,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「预览」图标，15×9：一只眼睛，上下两道圆弧围成眼眶，中间一个圆的瞳孔。
///
/// 图形按比例 `k` 缩放，线宽不变；出现、消失的动画中 `k` 逐渐变化。
pub fn eye(frame: &mut Frame<'_>, center: (f32, f32), k: f32, color: Color, clip: Rect) {
    draw(frame, Icon::Eye, center, 15.0 * k, RING_STROKE, color, clip);
}

/// 绘制「在浏览器中打开」图标，13×13：一个地球，外圈一个圆，中间一道竖着的经线和一道横着的
/// 赤道。
///
/// 图形按比例 `k` 缩放，线宽不变；出现、消失的动画中 `k` 逐渐变化。
pub fn globe(frame: &mut Frame<'_>, center: (f32, f32), k: f32, color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Globe,
        center,
        13.0 * k,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「Markdown」图标，15×11：圆角框，内有字母 M 和一个向下的箭头。
///
/// 用于设置的分类栏。
pub fn markdown(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Markdown,
        center,
        15.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「格式化」图标，14×10：几道长短、缩进各不相同的横线，表示排版后的代码。
///
/// 用于设置的分类栏。
pub fn format(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Format, center, 14.0, RING_STROKE, color, clip);
}

/// 绘制「终端」图标，15×11：圆角框，内有提示符 `>_`。
///
/// 用于设置的分类栏。
pub fn console(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Terminal,
        center,
        FRAME,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「主机」图标，13×12：上下两个圆角的机箱，各有一道短线表示指示灯。
///
/// 用于连接远端主机的入口和主机列表。
pub fn server(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Server, center, FRAME, RING_STROKE, color, clip);
}

/// 绘制「模块」图标，13×13：四个圆角小方块排成两行两列，表示可以拼装的部件。
///
/// 用于设置的分类栏。
pub fn modules(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Modules, center, 13.0, RING_STROKE, color, clip);
}

/// 绘制「关于」图标，约 12×12：一个圆圈，中间一个小写的「i」（上面一点、下面一竖），与
/// [`appearance`] 的圆同大。
///
/// 用于设置的分类栏。
pub fn about(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Info,
        center,
        2.0 * RING,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「快捷键」图标，15×11：圆角键盘外框，内有两排键和一道空格键。
///
/// 用于设置的分类栏。
pub fn keyboard(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Keyboard,
        center,
        15.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「设置」图标（齿轮），约 14×14：线条风格，一圈齿和中间的轴孔，笔宽 `STROKE`。
///
/// 欢迎页的「设置」一行使用它。
pub fn gear(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    // 齿顶半径 6.4，在图标集里是 9 格。
    draw(
        frame,
        Icon::Gear,
        center,
        fit(6.4, 9.0),
        STROKE,
        color,
        clip,
    );
}

/// 运行（三角形），约 9×11。
pub fn play(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Play, center, 13.0, RING_STROKE, color, clip);
}

/// 停止（方形），10×10。
pub fn stop(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Stop, center, 13.0, RING_STROKE, color, clip);
}
