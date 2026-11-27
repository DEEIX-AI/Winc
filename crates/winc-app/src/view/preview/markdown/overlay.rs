//! 预览叠加层：行号、分栏光标竖条、标题折叠箭头和选区行判断。

use super::*;

/// 在可编辑预览的左侧留白里写原文行号（见 `laid::source_lines`），光标所在的行颜色深一些。
///
/// 关闭行号设置时、只读的预览中不写；留白放不下最大的行号时也不写。
///
/// `skip` 是画了折叠箭头的行，不再写行号。
pub(super) fn draw_source_numbers(app: &App, frame: &mut Frame<'_>, area: Rect, skip: &[usize]) {
    if !app.markdown_line_numbers_on() || !app.markdown_live() {
        return;
    }
    let (Some(laid), Some(doc)) = (app.preview.markdown.layout.as_ref(), app.editor.active())
    else {
        return;
    };
    let Some(rope) = doc.text().rope() else {
        return;
    };
    let p = app.palette;
    let size = app.fonts.code;
    let cell = frame.measure(Font::mono(size), "0");
    let right = laid.left - drag::NUMBER_GAP;
    if laid.left - number_room(app, frame) < area.x + 4.0 {
        return;
    }
    let lit = selected_rows(laid, rope, &doc.selection);
    let top = drawn_scroll(app.preview.markdown.scroll()) - laid.pad;
    let lines = crate::app::preview::markdown::laid::source_lines(laid, rope, top..top + area.h);
    for (line, y) in lines {
        if skip.contains(&line) {
            continue;
        }
        let number = (line + 1).to_string();
        let x = right - number.len() as f32 * cell;
        // 行号只作定位参考，比正文的次要色更淡；光标或选区所在显示行的行号用次要色。
        let color = match lit.contains(line, y) {
            true => p.text_muted,
            false => p.text_dim,
        };
        let baseline = area.y + y - top;
        frame.text(Font::mono(size), (x, baseline), &number, color, area);
    }
}

/// 分栏时标出编辑区光标所在行的竖条（窗口坐标）：宽 2 逻辑像素，在内容左边的留白正中。
///
/// `(y, h)` 是光标的上边缘（内容坐标，含上边距）和高度，`top` 是内容坐标原点的窗口纵坐标。所在
/// 行是向左展开的表格时，竖条随表格的左边缘左移，不压在表格上。
pub(crate) fn caret_bar(laid: &Laid, (y, h): (f32, f32), top: f32) -> Rect {
    let spread = (laid.table_span(y + h / 2.0 - laid.pad)).map_or(0.0, |(x, _)| x.min(0.0));
    Rect::new(laid.left + spread - laid.pad / 2.0 - 1.0, top + y, 2.0, h)
}

/// 在标题左边画收起、展开下面一节的箭头：鼠标在标题上时显示，收起的标题一直显示，朝右；
/// 点击的范围记成链接（[`FOLD`]）。
///
/// 有行号时箭头画在这一行的行号上；没有行号时画在留白正中，与分栏时标出光标所在行的竖条
/// `bar` 同一列，竖条落在画了箭头的标题上时箭头改用强调色、不再画竖条。返回画了箭头的行，
/// 以及竖条是否被箭头代替。
pub(super) fn draw_heading_folds(
    app: &mut App,
    frame: &mut Frame<'_>,
    area: Rect,
    bar: Option<Rect>,
) -> (Vec<usize>, bool) {
    let (mut lines, mut covered) = (Vec::new(), false);
    let Some(laid) = app.preview.markdown.layout.as_ref() else {
        return (lines, covered);
    };
    let folds = (app.editor.active()).and_then(|d| app.preview.markdown.folds.get(&d.id));
    let scroll = drawn_scroll(app.preview.markdown.scroll());
    let (mx, my) = app.mouse();
    let p = app.palette;
    let numbers = app.markdown_line_numbers_on() && app.markdown_live();
    let x = drag::fold_center(laid.left, numbers, laid.pad);
    let view = scroll - laid.pad;
    for i in laid.blocks_in(view, view + area.h) {
        let Some(section) = laid.sections.get(i).and_then(Option::as_ref) else {
            continue;
        };
        let top = area.y - scroll + laid.pad + laid.tops[i];
        let height = laid.blocks[i].block.height;
        let folded = folds.is_some_and(|f| f.contains(&section.key));
        let over = area.contains(mx, my) && (top..top + height).contains(&my);
        if !(folded || over) {
            continue;
        }
        let start = laid.ranges[i].start;
        let cy = laid
            .caret_point(start)
            .map_or(top + height / 2.0, |(_, y, h)| {
                area.y - scroll + y + h / 2.0
            });
        let hovered = app.pointer.hover == Hover::MarkdownLink(app.hits.markdown_links.len());
        let marked = bar.is_some_and(|b| (b.y..b.bottom()).contains(&cy));
        covered |= marked;
        let color = match (marked, hovered) {
            (true, _) => p.accent,
            (false, true) => p.text,
            (false, false) => p.text_dim,
        };
        let turned = if folded {
            0.0
        } else {
            std::f32::consts::FRAC_PI_2
        };
        icons::chevron_turned(frame, (x, cy), ARROW, turned, color, area);
        let hit = Rect::new(x - ARROW - 2.0, cy - 10.0, 2.0 * ARROW + 4.0, 20.0);
        let url = format!("{FOLD}{}", section.key);
        app.hits.markdown_links.push((hit, url));
        if let Some(&line) = laid.lines.get(i) {
            lines.push(line);
        }
    }
    (lines, covered)
}

/// 返回第 `i` 块的上沿到下一块上沿的距离（逻辑像素）。
pub(crate) fn room_below(laid: &Laid, i: usize) -> f32 {
    let top = laid.tops[i];
    laid.tops
        .get(i + 1)
        .map_or(laid.blocks[i].block.height, |next| next - top)
}

/// 标题折叠箭头的大小（逻辑像素）。
const ARROW: f32 = drag::FOLD;

/// 选区 `selection` 在排版结果 `laid` 中占据的显示行；一行原文可能折成几个显示行，几行原文也
/// 可能排在同一显示行，因此按显示行判断，不按原文行号判断。
fn selected_rows(
    laid: &crate::app::preview::markdown::laid::Laid,
    rope: &ropey::Rope,
    selection: &winc_core::Selection,
) -> Lit {
    use crate::app::preview::markdown::laid::point_of;
    let (from, to) = (
        selection.anchor.min(selection.head),
        selection.anchor.max(selection.head),
    );
    let byte = |c: usize| rope.char_to_byte(c.min(rope.len_chars()));
    match (point_of(laid, byte(from)), point_of(laid, byte(to))) {
        (Some((_, a, _)), Some((_, b, h))) => Lit::Rows(a - laid.pad, b + h - laid.pad),
        _ => {
            let line = |c: usize| rope.char_to_line(c.min(rope.len_chars()));
            Lit::Lines(line(from), line(to))
        }
    }
}

/// 光标或选区占据的范围，决定哪些行号显示为当前行（见 [`draw_source_numbers`]）。
enum Lit {
    /// 显示行的上下范围（相对内容顶部，与 `laid::source_lines` 的基线同一量法）。
    Rows(f32, f32),
    /// 光标或选区的一端落在没有显示文字的位置（空行、分隔线）：按原文行号的范围判断。
    Lines(usize, usize),
}

impl Lit {
    /// 写在基线 `y` 处的原文第 `line` 行的行号是否在光标或选区所在的显示行上。
    fn contains(&self, line: usize, y: f32) -> bool {
        match *self {
            Lit::Rows(top, bottom) => (top..=bottom).contains(&y),
            Lit::Lines(first, last) => (first..=last).contains(&line),
        }
    }
}
