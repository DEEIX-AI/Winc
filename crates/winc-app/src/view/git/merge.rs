//! 在文件里解决合并冲突的绘制：冲突各部分的底色、标记行上的标题和按钮、右上角的卡片。
//!
//! 当前这一侧铺蓝、对方铺绿、共同祖先铺灰，和差异视图用同一组柔和色。标记行不显示原来的
//! `<<<<<<<`，换成一条标题：`<<<<<<<` 那条写「当前 · HEAD」，后面是「采用当前」「采用对方」
//! 「两者都要」「对比」，右端是第几处；`|||||||` 写「共同祖先」，`=======` 写「对方 · 分支名」，
//! `>>>>>>>` 只是一道收尾的细线。

use winc_editor::git::merge::{Conflict, Pick, SideAlign};
use winc_i18n::t;

use crate::app::git::merge::{MergeTarget, PANE_HEADER};
use crate::view::*;

/// 标记行上的按钮，从左到右；冲突序号用 `usize::MAX` 占位，绘制时换成当前冲突的序号。
const ACTIONS: [MergeTarget; 4] = [
    MergeTarget::Pick(usize::MAX, Pick::Ours),
    MergeTarget::Pick(usize::MAX, Pick::Theirs),
    MergeTarget::Pick(usize::MAX, Pick::Both),
    MergeTarget::Compare(usize::MAX),
];

/// 把 `ACTIONS` 里的占位换成第 `i` 段冲突。
fn for_conflict(target: MergeTarget, i: usize) -> MergeTarget {
    match target {
        MergeTarget::Pick(_, pick) => MergeTarget::Pick(i, pick),
        MergeTarget::Compare(_) => MergeTarget::Compare(i),
        other => other,
    }
}

fn label(target: MergeTarget) -> &'static str {
    match target {
        MergeTarget::Pick(_, pick) | MergeTarget::All(pick) => pick.label(),
        MergeTarget::Compare(_) => t!("git-merge-compare"),
        MergeTarget::Previous => t!("git-merge-previous"),
        MergeTarget::Next => t!("git-merge-next"),
        MergeTarget::MarkResolved => t!("git-merge-mark-resolved"),
        MergeTarget::ThreePane => t!("git-merge-three-pane"),
    }
}

/// 冲突里一行（不是标记行）的底色；`line` 不在冲突的内容里时为 `None`。
pub(in crate::view) fn tint(p: &Palette, conflict: &Conflict, line: usize) -> Option<Color> {
    use crate::app::git::merge::{Part, part_of};
    match part_of(conflict, line)? {
        Part::Ours => Some(p.diff_modified.line),
        Part::Theirs => Some(p.diff_added.line),
        Part::Base => Some(p.diff_deleted.line),
        Part::Marker => None,
    }
}

/// 绘制第 `i` 段冲突（共 `count` 段）的标记行 `line`，画在 `band`（文字区里这一行，逻辑像素）上。
#[allow(clippy::too_many_arguments)]
pub(in crate::view) fn draw_marker(
    app: &App,
    frame: &mut Frame<'_>,
    i: usize,
    count: usize,
    conflict: &Conflict,
    line: usize,
    band: Rect,
    hits: &mut Vec<(Rect, MergeTarget)>,
) {
    let p = app.palette;
    let side = |who: &str, name: &str| {
        if name.is_empty() {
            who.to_owned()
        } else {
            format!("{who} · {name}")
        }
    };
    let (color, title) = if line == conflict.start {
        (
            p.diff_modified.word,
            side(t!("git-merge-side-current"), &conflict.ours_label),
        )
    } else if Some(line) == conflict.base {
        (p.diff_deleted.word, t!("git-merge-side-base").to_owned())
    } else if line == conflict.middle {
        (
            p.diff_added.word,
            side(t!("git-merge-side-incoming"), &conflict.theirs_label),
        )
    } else {
        // `>>>>>>>`：盖住标记，只留一道收尾的细线。
        frame.rect_clipped(band, p.editor_bg, 0.0, band);
        let rule = Rect::new(band.x, (band.y + band.h / 2.0 - 1.0).round(), band.w, 2.0);
        frame.rect_clipped(rule, p.diff_added.word, 0.0, band);
        return;
    };
    frame.rect_clipped(band, color, 0.0, band);
    let mut x = ui_text_small(frame, app, band.x + TEXT_PAD, band, &title, p.text, band) + 14.0;
    if line != conflict.start {
        return;
    }

    // 「采用当前」「采用对方」「两者都要」「对比」，只有文字，悬停时铺底色。
    let h = (band.h - 4.0).max(12.0);
    for action in ACTIONS {
        let target = for_conflict(action, i);
        let text = label(target);
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + 12.0;
        let rect = Rect::new(x, band.y + (band.h - h) / 2.0, w, h);
        if app.hover == Hover::Merge(target) {
            frame.rect_clipped(rect, p.hover, RADIUS_ROW, band);
        }
        ui_text_small(frame, app, rect.x + 6.0, band, text, p.accent, band);
        hits.push((rect.intersect(band), target));
        x = rect.right() + 2.0;
    }

    // 右端：第几处。
    let counter = format!("{} / {count}", i + 1);
    let w = frame.measure(FontKind::Ui, app.fonts.ui_small, &counter);
    let right = band.right() - 14.0 - w;
    if right > x + 8.0 {
        ui_text_small(frame, app, right, band, &counter, p.text_muted, band);
    }
}

/// 绘制右上角的冲突卡片：还剩几处冲突、上一处下一处、全部采用一边；冲突都去掉了但还没标记为
/// 已解决时是「冲突都已解决」和「标记为已解决」。
///
/// 返回卡片的矩形（窗口坐标，逻辑像素），不需要卡片时为 `None`。
pub(in crate::view) fn draw_merge_card(
    app: &mut App,
    frame: &mut Frame<'_>,
    top: f32,
    hits: &mut Vec<(Rect, MergeTarget)>,
) -> Option<Rect> {
    let index = app.editor.active?;
    let count = app.conflicts(index).len();
    if count == 0 && !app.active_unmerged() {
        return None;
    }
    let p = app.palette;
    let editor = app.layout.editor;
    let font = app.fonts.ui;
    let panes = app.merge_panes_shown();
    let (message, buttons): (String, Vec<MergeTarget>) = if count > 0 {
        (
            t!("git-merge-card-conflicts", count = count),
            // 三栏时中间较窄，两侧已有采用的按钮，不再显示「全部采用」。
            if panes {
                vec![
                    MergeTarget::Previous,
                    MergeTarget::Next,
                    MergeTarget::ThreePane,
                ]
            } else {
                vec![
                    MergeTarget::Previous,
                    MergeTarget::Next,
                    MergeTarget::ThreePane,
                    MergeTarget::All(Pick::Ours),
                    MergeTarget::All(Pick::Theirs),
                ]
            },
        )
    } else {
        (
            t!("git-merge-card-resolved").to_owned(),
            vec![MergeTarget::MarkResolved],
        )
    };
    let widths: Vec<f32> = buttons
        .iter()
        .map(|&b| match b {
            MergeTarget::Previous | MergeTarget::Next => BUTTON_SMALL,
            b => frame.measure(FontKind::Ui, font, all_label(b, panes)) + 16.0,
        })
        .collect();
    let buttons_w: f32 = widths.iter().sum::<f32>() + 4.0 * (widths.len() - 1) as f32;
    let text_w = frame.measure(FontKind::Ui, font, &message);
    let width = 34.0 + text_w + 16.0 + buttons_w + 8.0;
    // 卡片高 36 逻辑像素（经验值）：比一行文字高，按钮上下各留一点空隙。
    let card = Rect::new(editor.right() - 8.0 - width, top, width, 36.0);

    draw_card(frame, p, card, RADIUS_BAR, p.popup_bg);
    let icon = (card.x + 16.0, icon_center_y(app, card, font));
    if count > 0 {
        icons::warning(frame, icon, p.warning, card);
    } else {
        icons::checkbox(
            frame,
            icon,
            Some(true),
            p.git_added,
            p.git_added,
            p.popup_bg,
            card,
        );
    }
    ui_text(frame, app, card.x + 34.0, card, &message, p.text, card);

    let mut x = card.right() - 8.0 - buttons_w;
    let y = card.y + (card.h - BUTTON_SMALL) / 2.0;
    for (&target, w) in buttons.iter().zip(widths) {
        let rect = Rect::new(x, y, w, BUTTON_SMALL);
        draw_button(
            frame,
            p,
            rect,
            card,
            app.hover == Hover::Merge(target),
            false,
        );
        match target {
            MergeTarget::Previous => {
                icons::chevron(
                    frame,
                    center(rect),
                    8.0,
                    icons::Direction::Up,
                    p.text_muted,
                    card,
                );
            }
            MergeTarget::Next => {
                icons::chevron(
                    frame,
                    center(rect),
                    8.0,
                    icons::Direction::Down,
                    p.text_muted,
                    card,
                );
            }
            _ => {
                ui_text(
                    frame,
                    app,
                    rect.x + 8.0,
                    rect,
                    all_label(target, panes),
                    p.accent,
                    card,
                );
            }
        }
        hits.push((rect, target));
        x += w + 4.0;
    }
    Some(card)
}

fn all_label(target: MergeTarget, panes: bool) -> &'static str {
    match target {
        MergeTarget::ThreePane if panes => t!("git-merge-close-three-pane"),
        MergeTarget::All(Pick::Ours) => t!("git-merge-accept-all-current"),
        MergeTarget::All(Pick::Theirs) => t!("git-merge-accept-all-incoming"),
        other => label(other),
    }
}

/// 三栏合并两侧按钮一列的宽度（逻辑像素）：与差异视图的按钮列相同。
const PANE_BUTTONS: f32 = 22.0;
/// 三栏合并两侧行号与文字的间距（逻辑像素）。
const PANE_PAD: f32 = 8.0;

/// 绘制三栏合并的两侧：标题、行号、代码、冲突的底色和采用按钮。
///
/// 三栏的标题行同高，两侧与中间的文档按 [`SideAlign`] 对齐：同一高度的行相对应。
pub(in crate::view) fn draw_side_panes(app: &mut App, frame: &mut Frame<'_>) {
    if app.merge.pane_rects[0].w <= 0.0 {
        return;
    }
    let Some(index) = app.editor.active else {
        return;
    };
    let conflicts = app.conflicts(index);
    let Some(mut panes) = app.merge.panes.take() else {
        return;
    };
    let doc = &app.editor.documents[index];
    let revision = doc.revision;
    if panes.aligned.as_ref().is_none_or(|(at, _)| *at != revision)
        && let Some(rope) = doc.text().rope()
    {
        let side = |side: &winc_editor::diff_view::DiffSide, pick| {
            SideAlign::new(rope, &conflicts, &side.text.to_string(), pick)
        };
        panes.aligned = Some((
            revision,
            [
                side(&panes.sides[0], Pick::Ours),
                side(&panes.sides[1], Pick::Theirs),
            ],
        ));
    }
    let doc = &app.editor.documents[index];
    let scroll = doc.scroll_line.max(0.0);
    let line = doc.line_of_row(scroll.floor() as usize);
    let fraction = scroll.fract();
    // 中间一栏的标题行：盖住滚过去的文字，冲突卡片随后画在上面。
    let editor = app.layout.editor;
    let header = Rect::new(editor.x, editor.y, editor.w, PANE_HEADER);
    let p = app.palette;
    frame.rect(header, p.editor_bg);
    let title = t!("git-merge-side-result");
    ui_text(
        frame,
        app,
        header.x + 12.0,
        header,
        title,
        p.text_muted,
        header,
    );
    frame.rect(
        Rect::new(header.x, header.bottom() - 1.0, header.w, 1.0),
        p.separator,
    );
    let mut hits = Vec::new();
    for side in 0..2 {
        let rect = app.merge.pane_rects[side];
        let Some((_, aligned)) = &panes.aligned else {
            continue;
        };
        let top = aligned[side].side_line(line) as f32 + fraction;
        draw_side_pane(
            app,
            frame,
            rect,
            side,
            &mut panes.sides[side],
            &aligned[side],
            top,
            &mut hits,
        );
    }
    app.merge.hits.extend(hits);
    app.merge.panes = Some(panes);
}

/// 绘制三栏合并的一侧：`top` 是标题行下方第一行的行号（可以是小数）。
#[allow(clippy::too_many_arguments)]
fn draw_side_pane(
    app: &mut App,
    frame: &mut Frame<'_>,
    rect: Rect,
    side: usize,
    source: &mut winc_editor::diff_view::DiffSide,
    aligned: &SideAlign,
    top: f32,
    hits: &mut Vec<(Rect, MergeTarget)>,
) {
    let p = app.palette;
    frame.rect(rect, p.editor_bg);
    let header = Rect::new(rect.x, rect.y, rect.w, PANE_HEADER);
    let body = Rect::new(
        rect.x,
        header.bottom(),
        rect.w,
        (rect.h - PANE_HEADER).max(0.0),
    );
    let (ascent, descent) = (app.mono_metrics.ascent, app.mono_metrics.descent);
    let lh = app.fonts.line_height;
    let total = source.text.len_lines();
    let digits = total.to_string().len().max(2) as f32;
    let gutter_w = (digits * app.cell + PANE_PAD * 2.0).round();
    // 左栏的按钮靠中间，在行号右边；右栏的按钮靠中间，在行号左边。
    let (buttons, gutter, text) = if side == 0 {
        let buttons = Rect::new(rect.right() - PANE_BUTTONS, body.y, PANE_BUTTONS, body.h);
        let gutter = Rect::new(buttons.x - gutter_w, body.y, gutter_w, body.h);
        (
            buttons,
            gutter,
            Rect::new(rect.x, body.y, (gutter.x - rect.x).max(0.0), body.h),
        )
    } else {
        let buttons = Rect::new(rect.x, body.y, PANE_BUTTONS, body.h);
        let gutter = Rect::new(buttons.right(), body.y, gutter_w, body.h);
        let x = gutter.right();
        (
            buttons,
            gutter,
            Rect::new(x, body.y, (rect.right() - x).max(0.0), body.h),
        )
    };
    let shade = if side == 0 {
        p.diff_modified.line
    } else {
        p.diff_added.line
    };
    let row_y = |line: usize| body.y + (line as f32 - top) * lh;

    // 冲突的底色和 `git` 自动合入的行的淡底色。
    for range in aligned.changed() {
        let band = Rect::new(rect.x, row_y(range.start), rect.w, range.len() as f32 * lh);
        frame.rect_clipped(band, p.hover, 0.0, body);
    }
    for block in &aligned.blocks {
        let band = Rect::new(rect.x, row_y(block.start), rect.w, block.len() as f32 * lh);
        frame.rect_clipped(band, shade, 0.0, body);
    }

    let first = top.floor().max(0.0) as usize;
    let visible = (body.h / lh).ceil() as usize + 1;
    let (mut chars, mut tokens) = (Vec::with_capacity(128), Vec::with_capacity(16));
    let tab_width = app.editor.tab_width;
    for line in first..(first + visible).min(total) {
        let band = Rect::new(rect.x, row_y(line), rect.w, lh);
        let baseline = baseline_in(band, ascent, descent);
        let number = (line + 1).to_string();
        let number_x = gutter.right() - PANE_PAD - number.chars().count() as f32 * app.cell;
        frame.text(
            FontKind::Mono,
            app.fonts.code,
            number_x,
            baseline,
            &number,
            p.gutter_text,
            gutter,
        );
        crate::view::diff::line_chars(source, line, &mut chars);
        let state =
            source
                .highlighter
                .state_at(&source.text, line, crate::view::editor::HIGHLIGHT_BUDGET);
        app.highlight_pending |= state.is_none();
        tokens.clear();
        let _ = winc_syntax::highlight_line(
            source.highlighter.language(),
            &chars,
            state.unwrap_or_default(),
            &mut tokens,
        );
        crate::view::editor::draw_glyphs(
            app,
            frame,
            &chars,
            &tokens,
            text.x + PANE_PAD,
            baseline,
            tab_width,
            text,
        );
    }

    // 每段冲突第一行旁边一个采用按钮。
    let pick = if side == 0 { Pick::Ours } else { Pick::Theirs };
    for (i, block) in aligned.blocks.iter().enumerate() {
        let mid = row_y(block.start) + lh / 2.0;
        if mid < body.y || mid > body.bottom() {
            continue;
        }
        let c = (buttons.x + buttons.w / 2.0, mid);
        let target = MergeTarget::Pick(i, pick);
        let hover = (app.hover == Hover::Merge(target)).then_some(p.hover);
        if side == 0 {
            icons::line_replace(frame, c, hover, p.accent, body);
        } else {
            icons::line_replace_back(frame, c, hover, p.accent, body);
        }
        hits.push((
            Rect::new(c.0 - 10.0, c.1 - 10.0, 20.0, 20.0).intersect(body),
            target,
        ));
    }

    // 标题行和与中间的分隔线。
    frame.rect(header, p.editor_bg);
    let title =
        ellipsize(frame, &source.title, app.fonts.ui, (rect.w - 24.0).max(0.0)).into_owned();
    ui_text(
        frame,
        app,
        rect.x + 12.0,
        header,
        &title,
        p.text_muted,
        header,
    );
    frame.rect(
        Rect::new(rect.x, header.bottom() - 1.0, rect.w, 1.0),
        p.separator,
    );
    let edge = if side == 0 {
        rect.right() - 1.0
    } else {
        rect.x
    };
    frame.rect(Rect::new(edge, rect.y, 1.0, rect.h), p.separator);
}
