//! 名字旁边的提示气泡：一张小卡片，带一个指向名字的小三角。
//!
//! 名字旁的提示与诊断气泡都挂在文字所在行的上方，上方放不下时翻到下方；横向夹在编辑区
//! 内。出现时滑出并淡入，消失时原地淡出，与位置浮窗的过渡一致。

use winc_render::Stroke;

use crate::app::editor::hint::{HintLine, HintPoint};

use super::*;

/// 单行气泡的高度（逻辑像素）：一行文字加下上留白（摆放测试用）。
#[cfg(test)]
pub(super) const HEIGHT: f32 = 36.0;
/// 文字左右的留白（逻辑像素）。
pub(super) const PAD: f32 = 14.0;
/// 文字上下的留白（逻辑像素）：消息框里文字的上下边距。
const PAD_V: f32 = 8.0;
/// 小三角的高度（逻辑像素），底边宽为它的两倍。
const POINTER: f32 = 6.0;
/// 小三角尖端与名字所在行之间的距离（逻辑像素）。
const GAP: f32 = 2.0;
/// 出现时滑动的距离（逻辑像素）。
const SLIDE: f32 = 3.0;
/// 气泡的最小宽度（逻辑像素）：再窄就读不成句。
const MIN_WIDTH: f32 = 180.0;
/// 气泡最多占编辑区宽度的比例。
const MAX_SHARE: f32 = 0.6;
/// 诊断消息最多显示的行数，超出截断并加省略号。
const MAX_LINES: usize = 6;
/// 每行文字的最小高度（逻辑像素）：随界面字号放大，至少留出这么高。
const LINE_H: f32 = 20.0;
/// 图标的最小尺寸与图标列的最小宽度（逻辑像素）：图标在文字左侧，`ICON_ROOM` 是图标加它
/// 与文字的间距。
const ICON_SIZE: f32 = 14.0;
pub(super) const ICON_ROOM: f32 = 21.0;

/// 气泡宽度：内容宽夹在 [`MIN_WIDTH`] 与编辑区宽度的 [`MAX_SHARE`] 之间，外加两侧留白。
pub(in crate::view) fn bubble_width(
    content: f32,
    padding: f32,
    editor: Rect,
) -> f32 {
    let max = (editor.w * MAX_SHARE).max(MIN_WIDTH);
    (content + padding).clamp(MIN_WIDTH, max)
}

/// 把诊断消息按 `measure` 的宽度折成若干行，最多 [`MAX_LINES`] 行；末行放不下时截断并加省略号。
///
/// 消息里的换行（多条诊断以 `\n` 连接）照旧分段；段内优先在空格处断开，单词比行宽还长时硬切。
pub(in crate::view) fn wrap_message(
    text: &str,
    mut measure: impl FnMut(&str) -> f32,
    width: f32,
    max_lines: usize,
) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) <= width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // 单词本身超宽：按字符硬切。
            let mut rest = word;
            while measure(rest) > width && !rest.is_empty() {
                let mut cut = rest.chars().count();
                while cut > 0 && measure(&rest.chars().take(cut).collect::<String>()) > width {
                    cut -= 1;
                }
                let cut = cut.max(1);
                lines.push(rest.chars().take(cut).collect());
                rest = rest.chars().skip(cut).collect::<String>().leak();
            }
            line = rest.to_owned();
        }
        lines.push(line);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            last.push('…');
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// 绘制名字旁的提示气泡；没有气泡、它挂的文档不是当前文档，或名字不在屏幕上（包括编辑区被
/// 铺满的终端面板遮住）时不绘制。
pub(in crate::view) fn draw_name_hint(app: &mut App, frame: &mut Frame<'_>) {
    let Some(hint) = &app.jumps.name_hint else {
        return;
    };
    if app.overlay_area() != app.layout.editor {
        return;
    }
    if app.editor.active().map(|d| d.id) != Some(hint.doc) {
        return;
    }
    let editor = app.layout.editor;
    let line_h = app.fonts.line_height;
    let Some((name_x, top)) = app
        .anchor_point(hint.anchor)
        .filter(|&(_, y)| y >= editor.y && y + line_h <= editor.bottom())
    else {
        return;
    };

    let p = app.palette;
    // 消息用界面字号显示：与界面里其它文字一致。不跟编辑区字号放大——编辑区字号较大时
    // 气泡文字会显得比代码还大。行高与上下留白仍随字号放大。
    let size = app.fonts.ui;
    let row_h = (size * 1.55).round().max(LINE_H);
    let pad_v = (size * 0.6).round().max(PAD_V);
    let icon_size = (size * 1.1).round().max(ICON_SIZE);
    let icon_room = (icon_size + 7.0).max(ICON_ROOM);
    // 诊断气泡：图标列 + 限宽折行的多行消息；名字旁的提示仍是一行。
    let (w, card_h, lines, icon) = if hint.diagnostic {
        let room = icon_room + 2.0 * PAD;
        let content = frame.measure(FontKind::Ui, size, &hint.text);
        let w = bubble_width(content, room, editor);
        let lines = wrap_message(
            &hint.text,
            |s| frame.measure(FontKind::Ui, size, s),
            w - room,
            MAX_LINES,
        );
        let card_h = lines.len() as f32 * row_h + 2.0 * pad_v;
        (w, card_h, lines, hint_level(app).map(|level| (level.glyph(), level.color(p))))
    } else {
        let w = frame.measure(FontKind::Ui, size, &hint.text) + 2.0 * PAD;
        (w, row_h + 2.0 * pad_v, vec![hint.text.clone()], None)
    };
    // 气泡锚定在文字处：诊断气泡挂在波浪线所在行的上方，名字旁的提示挂在名字上方。
    let (x, y, above) = place_above_name(name_x, top, line_h, editor, w, card_h);

    // 出现：不透明度二次缓出，位置三次缓出；消失：原地淡出，二次缓入。
    let (opacity, away) = match (hint.fade_in, hint.fade_out) {
        (_, Some(out)) => {
            let t = out.linear();
            (1.0 - t * t, 0.0)
        }
        (Some(fade_in), None) => {
            let t = fade_in.linear();
            (1.0 - (1.0 - t).powi(2), (1.0 - t).powi(3))
        }
        (None, None) => (1.0, 0.0),
    };
    let toward_name = if above { 1.0 } else { -1.0 };
    let card = Rect::new(x, (y + toward_name * SLIDE * away).round(), w, card_h);
    let tip_x = (name_x + 6.0)
        .clamp(card.x + 16.0, (card.right() - 16.0).max(card.x + 16.0))
        .round();

    // 鼠标从诊断文字移向气泡时要经过卡片与文字之间的空隙：把这个小三角的范围也一并算作
    // 「在气泡上」，鼠标才不会在半路把气泡淡出。
    let bridge = if above {
        Rect::new(tip_x - POINTER, card.bottom(), 2.0 * POINTER, POINTER + GAP)
    } else {
        Rect::new(
            tip_x - POINTER,
            card.y - POINTER - GAP,
            2.0 * POINTER,
            POINTER + GAP,
        )
    };
    let hover = Rect::new(
        card.x.min(bridge.x),
        card.y.min(bridge.y),
        card.right().max(bridge.right()) - card.x.min(bridge.x),
        card.bottom().max(bridge.bottom()) - card.y.min(bridge.y),
    );

    let selection = app.jumps.hint_selection;
    let mut drawn: Vec<HintLine> = Vec::with_capacity(lines.len());
    frame.with_opacity(opacity, |frame| {
        draw_card(frame, p, card, RADIUS_POPUP, p.popup_bg);
        draw_pointer(frame, p, card, tip_x, above);
        let mut text_x = card.x + PAD;
        if let Some((glyph, color)) = icon {
            // 图标与第一行文字的中心对齐：首行从卡片上留白 `pad_v` 处开始，与下面的文字行相同。
            let first = Rect::new(text_x, card.y + pad_v, icon_room, row_h);
            draw_level_icon(frame, app, first, size, glyph, color);
            text_x += icon_room;
        }
        for (i, line) in lines.iter().enumerate() {
            let row = Rect::new(
                text_x - PAD,
                card.y + pad_v + i as f32 * row_h,
                card.w - (text_x - card.x) - PAD,
                row_h,
            );
            // 记录每个字符的右边界，供拖选与复制按位置找字。
            let mut edges = Vec::with_capacity(line.chars().count());
            let mut right = text_x;
            let mut buf = [0u8; 4];
            for c in line.chars() {
                right += frame.measure(FontKind::Ui, size, c.encode_utf8(&mut buf));
                edges.push(right);
            }
            if let Some((from, to)) = selected_chars(selection, i, edges.len()) {
                let left = if from == 0 { text_x } else { edges[from - 1] };
                let band = Rect::new(left, row.y + 1.0, edges[to - 1] - left, row.h - 2.0);
                frame.rect_clipped(band, p.selection, 2.0, card);
            }
            label(frame, app, size, (text_x, row), line, p.text, card);
            drawn.push(HintLine {
                row,
                x: text_x,
                text: line.clone(),
                edges,
            });
        }
    });
    app.hits.hint_card = Some(card);
    app.hits.hint_hover = Some(hover);
    app.hits.hint_lines = drawn;
}

/// 第 `line` 行在气泡选区 `selection` 中选中的字符范围（`len` 是这一行的字符数）；没选中时
/// 为 `None`。
fn selected_chars(
    selection: Option<(HintPoint, HintPoint)>,
    line: usize,
    len: usize,
) -> Option<(usize, usize)> {
    let (a, b) = selection?;
    let (from, to) = if a <= b { (a, b) } else { (b, a) };
    if line < from.line || line > to.line {
        return None;
    }
    let start = if line == from.line {
        from.ch.min(len)
    } else {
        0
    };
    let end = if line == to.line { to.ch.min(len) } else { len };
    (start < end).then_some((start, end))
}

/// 诊断气泡的严重级别：从消息里带的信息取出（多条合并时取最严重的）。
fn hint_level(app: &App) -> Option<Level> {
    app.jumps
        .name_hint
        .as_ref()
        .and_then(|h| h.level)
        .and_then(Level::of)
}

/// 诊断的最高严重级别：决定气泡左侧图标的形状与颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::view) enum Level {
    Error,
    Warning,
}

impl Level {
    /// 从 LSP 的严重级别换算（0 = 错误，1 = 警告）。
    pub(in crate::view) fn of(severity: u32) -> Option<Level> {
        match severity {
            0 => Some(Level::Error),
            1 => Some(Level::Warning),
            _ => None,
        }
    }

    fn glyph(self) -> LevelGlyph {
        match self {
            Level::Error => LevelGlyph::Cross,
            Level::Warning => LevelGlyph::Triangle,
        }
    }

    fn color(self, p: &Palette) -> Color {
        match self {
            Level::Error => p.error,
            Level::Warning => p.warning,
        }
    }
}

/// 严重级别的图标形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LevelGlyph {
    /// 错误：实心圆里的叉。
    Cross,
    /// 警告：三角形里的感叹号竖线加点。
    Triangle,
}

/// 在 `row` 的左侧画严重级别图标，与字号为 `size` 的行内文字的视觉中心对齐。
fn draw_level_icon(
    frame: &mut Frame<'_>,
    app: &App,
    row: Rect,
    size: f32,
    glyph: LevelGlyph,
    color: Color,
) {
    let icon = (size * 1.1).round().max(ICON_SIZE);
    let cx = row.x + icon / 2.0 + 1.0;
    let cy = icon_center_y(app, row, size);
    let r = icon / 2.0;
    match glyph {
        LevelGlyph::Cross => {
            frame.stroke(&[Stroke::Circle(cx, cy, r)], 1.3, color, row);
            let k = r * 0.42;
            frame.stroke(
                &[
                    Stroke::Line(cx - k, cy - k, cx + k, cy + k),
                    Stroke::Line(cx + k, cy - k, cx - k, cy + k),
                ],
                1.3,
                color,
                row,
            );
        }
        LevelGlyph::Triangle => {
            let top = cy - r;
            let bottom = cy + r;
            let half = r * 1.05;
            frame.stroke(
                &[
                    Stroke::Line(cx, top, cx + half, bottom),
                    Stroke::Line(cx + half, bottom, cx - half, bottom),
                    Stroke::Line(cx - half, bottom, cx, top),
                ],
                1.3,
                color,
                row,
            );
            // 感叹号：一根竖线与下面的一个点。
            frame.stroke(
                &[Stroke::Line(cx, cy - r * 0.35, cx, cy + r * 0.05)],
                1.3,
                color,
                row,
            );
            frame.stroke(
                &[Stroke::Line(cx, cy + r * 0.42, cx, cy + r * 0.42)],
                1.8,
                color,
                row,
            );
        }
    }
}

/// 锚定文字的提示气泡的摆放：文字所在行的上方，上方放不下时放到行下方；`height` 是卡片
/// 的实际高度（诊断消息折行后不止一行）。
pub(in crate::view) fn place_above_name(
    name_x: f32,
    top: f32,
    line_h: f32,
    editor: Rect,
    width: f32,
    height: f32,
) -> (f32, f32, bool) {
    let right = editor.right() - SCROLLBAR_HIT - 8.0;
    let x = (name_x - 20.0).min(right - width).max(editor.x + 8.0).round();
    let above_y = top - GAP - POINTER - height;
    let below_y = top + line_h + GAP + POINTER;
    let fits_above = above_y >= editor.y + 4.0;
    let fits_below = below_y + height <= editor.bottom() - 4.0;
    let y = if fits_above {
        above_y
    } else if fits_below {
        below_y
    } else if top - editor.y >= editor.bottom() - top {
        // 两侧都放不下：取空间较大的一侧，再夹在编辑区内。
        above_y
    } else {
        below_y
    };
    let y = y.clamp(editor.y + 4.0, (editor.bottom() - height - 4.0).max(editor.y + 4.0)).round();
    let is_above = y + height <= top;
    (x, y, is_above)
}

/// 在卡片 `card` 的下边（`down` 为真）或上边绘制一个尖端横坐标为 `tip_x` 的小三角，填充与
/// 卡片相同，两条斜边描上卡片的边框色；三角的底边遮住卡片在那一段的边框，与卡片连成一体。
fn draw_pointer(frame: &mut Frame<'_>, p: &Palette, card: Rect, tip_x: f32, down: bool) {
    let edge = if down { card.bottom() } else { card.y };
    let dir = if down { 1.0 } else { -1.0 };
    // 每半个逻辑像素一条，自底边向尖端逐条收窄；第一条压住卡片的边框。
    let steps = (POINTER * 2.0) as usize;
    for i in 0..=steps {
        let d = i as f32 * 0.5;
        let half = POINTER - d;
        let y = if down { edge + d - 1.0 } else { edge - d + 0.5 };
        let strip = Rect::new(tip_x - half, y, 2.0 * half, 0.5);
        frame.rect(strip, p.popup_bg);
    }
    let tip = (tip_x, edge + dir * POINTER);
    let lines = [
        Stroke::Line(tip_x - POINTER, edge, tip.0, tip.1),
        Stroke::Line(tip.0, tip.1, tip_x + POINTER, edge),
    ];
    frame.stroke(&lines, 1.0, p.popup_border, frame.bounds());
}
