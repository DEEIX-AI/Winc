//! 差异中的代码折叠与括号配对：行号栏的折叠箭头、收起后行尾的标记、光标旁括号的底色。

use winc_core::movement;

use super::*;

/// 光标旁括号与配对括号之间最多扫描的字符数，与编辑区相同的量级，长文件中不阻塞一帧。
const BRACE_SCAN: usize = 20_000;

/// 返回差异光标旁的括号及其配对括号：（侧，（两个字符偏移））。
pub(super) fn matching_brace(view: &DiffView, edit: &EditState) -> Option<(usize, (usize, usize))> {
    let (side, (line, col)) = if edit.editing {
        (RIGHT, edit.caret)
    } else {
        edit.read_caret?
    };
    let text = &view.side(side).text;
    let pos = text.line_col_to_pos(line, col);
    movement::matching_bracket(text, pos, BRACE_SCAN).map(|pair| (side, pair))
}

/// 返回两侧本帧绘制的行中是代码块开头的行（升序）。
///
/// 展开的块只在鼠标位于该侧行号栏时显示箭头，其余时候不必计算。
pub(super) fn foldable_lines(
    app: &App,
    view: &mut DiffView,
    parts: &DiffParts,
    drawn: &[DrawnRow],
) -> [Vec<usize>; 2] {
    let (x, y) = app.mouse_position();
    let tab_width = app.editor.tab_width;
    [LEFT, RIGHT].map(|side| {
        if !parts.panes[side].gutter.contains(x, y) {
            return Vec::new();
        }
        let mut lines: Vec<usize> = (drawn.iter())
            .filter(|row| row.side == side)
            .filter_map(|row| view.line_at(side, row.row))
            .collect();
        lines.dedup();
        view.code_fold_starts(side, lines, tab_width)
    })
}

/// 在第 `side` 侧第 `line` 行的行号栏画折叠箭头：下面有收起的块时始终显示（向右），`starts`
/// 为真（该行是块的开头，鼠标在行号栏）时显示向下的箭头。左侧画在行号之前，右侧画在行号之后。
pub(super) fn draw_arrow(
    app: &App,
    frame: &mut Frame<'_>,
    view: &DiffView,
    (pane, body): (&DiffPane, Rect),
    (side, line, starts): (usize, usize, bool),
    y: f32,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let collapsed = view.collapsed_code_fold(side, line).is_some();
    if !collapsed && !starts {
        return;
    }
    let lh = app.fonts.line_height;
    let pad = crate::app::diff::DIFF_GUTTER_PAD;
    let x = if side == LEFT {
        pane.gutter.x
    } else {
        pane.gutter.right() - pad
    };
    let target = DiffTarget::CodeFold(side, line);
    let mark = Rect::new(x, y, pad, lh);
    let p = app.palette;
    let color = if app.pointer.hover == Hover::Diff(target) {
        p.text_muted
    } else {
        p.text_dim
    };
    let angle = if collapsed {
        0.0
    } else {
        std::f32::consts::FRAC_PI_2
    };
    let center = (mark.x + mark.w / 2.0, y + lh / 2.0);
    icons::chevron_turned(frame, center, 5.0, angle, color, pane.gutter);
    hits.push((mark.intersect(body), target));
}

/// 在第 `side` 侧第 `line` 行末尾、`(x, y)` 处画收起标记（三个点），点击后展开。
pub(super) fn draw_ellipsis(
    app: &App,
    frame: &mut Frame<'_>,
    (side, line): (usize, usize),
    (x, y): (f32, f32),
    clip: Rect,
    hits: &mut Vec<(Rect, DiffTarget)>,
) {
    let p = app.palette;
    let lh = app.fonts.line_height;
    let chip = Rect::new(x, y + 3.0, 22.0, lh - 6.0);
    let target = DiffTarget::CodeFold(side, line);
    frame.rect_clipped(chip, 4.0, p.selected_inactive, clip);
    if app.pointer.hover == Hover::Diff(target) {
        frame.rect_clipped(chip, 4.0, p.hover, clip);
    }
    let cy = (chip.y + chip.h / 2.0 - 1.0).round();
    for i in 0..3 {
        let dot = Rect::new(chip.x + 6.0 + i as f32 * 4.0, cy, 2.0, 2.0);
        frame.rect_clipped(dot, 1.0, p.text_muted, clip);
    }
    hits.push((chip.intersect(clip), target));
}
