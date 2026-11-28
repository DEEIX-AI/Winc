//! 查找和全局搜索的按钮：勾选框、行内的替换和勾选、筛选、替换、正则、整词。

use super::*;

/// 差异视图中间一栏按钮的悬停底色：与工具栏的图标按钮相同，悬停时才铺一层中性的底，平时
/// 只有图标；按钮叠在改动的底色上，一直铺淡色方块会与背景混同，看不出按钮边界。
fn line_button_hover(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    hover: Option<Color>,
    clip: Rect,
) {
    if let Some(hover) = hover {
        let rect = Rect::new((cx - 10.0).round(), (cy - 10.0).round(), 20.0, 20.0);
        frame.rect_clipped(rect, 4.0, hover, clip);
    }
}

/// 绘制差异视图里的勾选框，线条风格，与旁边的「»」一致：边长 `size` 的圆角细框，勾上时
/// 框和勾都是强调色，不铺底色；没勾时只有一圈淡色的框。
///
/// `hover` 是悬停时的底色。
#[allow(clippy::too_many_arguments)]
pub fn line_checkbox(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    on: bool,
    hover: Option<Color>,
    accent: Color,
    border: Color,
    clip: Rect,
) {
    line_button_hover(frame, center, hover, clip);
    let rect = checkbox_rect(center, size);
    let outline_color = if on { accent } else { border };
    frame.stroke(&checkbox_outline(rect), 1.2, outline_color, clip);
    if on {
        let k = size / 12.0;
        let (x, y) = (rect.x, rect.y);
        let mark = [
            Stroke::Line(x + 3.2 * k, y + 6.2 * k, x + 5.2 * k, y + 8.2 * k),
            Stroke::Line(x + 5.2 * k, y + 8.2 * k, x + 8.9 * k, y + 4.0 * k),
        ];
        frame.stroke(&mark, MARK_STROKE, accent, clip);
    }
}

/// 绘制只包含部分内容的勾选框：样式与 [`line_checkbox`] 勾上时相同，框内是一条横线。
///
/// `hover` 是悬停时的底色。
pub fn line_checkbox_mixed(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    size: f32,
    hover: Option<Color>,
    accent: Color,
    clip: Rect,
) {
    line_button_hover(frame, center, hover, clip);
    let rect = checkbox_rect(center, size);
    frame.stroke(&checkbox_outline(rect), 1.2, accent, clip);
    let (inset, y) = (size * 0.25, rect.y + rect.h / 2.0);
    let mark = [Stroke::Line(rect.x + inset, y, rect.right() - inset, y)];
    frame.stroke(&mark, MARK_STROKE, accent, clip);
}

/// 绘制三栏合并右栏的「«」按钮：把右栏这一段采用到中间，样式与 [`line_replace`] 相同，方向相反。
///
/// `hover` 是悬停时的底色。
pub fn line_replace_back(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    hover: Option<Color>,
    color: Color,
    clip: Rect,
) {
    line_button_hover(frame, center, hover, clip);
    draw(
        frame,
        Icon::ChevronsLeft,
        center,
        LINE_REPLACE,
        MARK_STROKE,
        color,
        clip,
    );
}

/// 绘制差异视图里的「»」按钮（用左侧替换这一段）：两道朝右的箭头，强调色，笔宽和勾选框的勾
/// 一样，大小和勾选框的框相当。
///
/// `hover` 是悬停时的底色。
pub fn line_replace(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    hover: Option<Color>,
    color: Color,
    clip: Rect,
) {
    line_button_hover(frame, center, hover, clip);
    draw(
        frame,
        Icon::ChevronsRight,
        center,
        LINE_REPLACE,
        MARK_STROKE,
        color,
        clip,
    );
}

/// 「»」「«」按钮的图标尺寸（逻辑像素）：箭头约 9×8，与勾选框的框相当。
const LINE_REPLACE: f32 = 12.0;

/// 勾选框（[`checkbox`]、[`line_checkbox`]）的方框：边长 `size`，以
/// `center` 为中心，左上角取整到整像素。
fn checkbox_rect((cx, cy): (f32, f32), size: f32) -> Rect {
    Rect::new(
        (cx - size / 2.0).round(),
        (cy - size / 2.0).round(),
        size,
        size,
    )
}

/// 勾选框的细框：画在方框 `rect` 里面半个像素，圆角 3。
fn checkbox_outline(rect: Rect) -> [Stroke; 8] {
    Stroke::rounded_rect(
        Rect::new(rect.x + 0.5, rect.y + 0.5, rect.w - 1.0, rect.h - 1.0),
        3.0,
    )
}

/// 12×12 勾选框里的记号：勾上（`full`）时是一个勾，只勾了一部分时是一道横线。
fn checkbox_mark(rect: Rect, full: bool) -> Strokes<2> {
    let (x, y) = (rect.x, rect.y);
    let mut mark = Strokes::new();
    if full {
        mark.extend([
            Stroke::Line(x + 3.0, y + 6.2, x + 5.2, y + 8.4),
            Stroke::Line(x + 5.2, y + 8.4, x + 9.2, y + 3.8),
        ]);
    } else {
        mark.push(Stroke::Line(x + 3.0, y + 6.0, x + 9.0, y + 6.0));
    }
    mark
}

/// 绘制勾选框，12×12，圆角方框：勾上时铺强调色、画白色的勾；`partial` 时（一组里只勾了一部分）
/// 铺强调色、画一道横线；没勾时只有一圈边框。
pub fn checkbox(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    state: Option<bool>,
    accent: Color,
    border: Color,
    on_accent: Color,
    clip: Rect,
) {
    let rect = checkbox_rect(center, 12.0);
    match state {
        None => frame.stroke(&checkbox_outline(rect), 1.0, border, clip),
        Some(full) => {
            frame.rect_clipped(rect, 3.0, accent, clip);
            let mark = checkbox_mark(rect, full);
            frame.stroke(mark.as_slice(), MARK_STROKE, on_accent, clip);
        }
    }
}

/// 绘制快速打开面板中的放大镜图标，约 12×12：线条版本，不带实心底色。
pub fn magnifier(frame: &mut Frame<'_>, (cx, cy): (f32, f32), color: Color, clip: Rect) {
    // 与 [`search`] 同一个放大镜，圆框略向左上偏。
    let (icon, extent) = (Icon::Search, fit(11.5, 15.5));
    draw(frame, icon, (cx - 0.5, cy - 0.5), extent, 1.5, color, clip);
}

/// 绘制「清除搜索结果」图标，12×12：三行文字，右下角叠着一个叉。
pub fn clear_results(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::ClearResults,
        center,
        12.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「过滤」图标，12×11：漏斗，上沿一道横线向下收窄为一根短柄。
pub fn filter(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Filter, center, 12.0, RING_STROKE, color, clip);
}

/// 绘制「被忽略的文件」图标，约 12×12：一个文件，被一条斜线划过。
pub fn ignored_files(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::IgnoredFiles,
        center,
        12.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「全部展开」图标，约 12×12，与「全部折叠」成对：卡片相同，卡片内是加号。
pub fn expand_all(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::ExpandAll,
        center,
        12.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「替换」图标，12×12：一个圆角框（表示输入框中的文字），一支箭头从右上方横向伸来，
/// 拐弯后向下指进框内。
pub fn replace_one(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Replace, center, 12.0, RING_STROKE, color, clip);
}

/// 绘制「全部替换」图标，12×12：与「替换」相同，框下方多一道线，表示叠放。
pub fn replace_all(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::ReplaceAll,
        center,
        12.0,
        RING_STROKE,
        color,
        clip,
    );
}

/// 绘制「正则」图标，约 10×11：左下一个点，右上一个星号，即「.*」。
pub fn regex(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    draw(frame, Icon::Regex, center, 14.0, GLYPH_STROKE, color, clip);
}

/// 绘制全字匹配开关中文字下方的托架：一条横线，两端向上翘，宽度为 `width`。
pub fn word_bracket(
    frame: &mut Frame<'_>,
    (cx, y): (f32, f32),
    width: f32,
    color: Color,
    clip: Rect,
) {
    let (left, right) = (cx - width / 2.0, cx + width / 2.0);
    let lines = [
        Stroke::Line(left, y - 2.5, left, y),
        Stroke::Line(left, y, right, y),
        Stroke::Line(right, y, right, y - 2.5),
    ];
    frame.stroke(&lines, GLYPH_STROKE, color, clip);
}
