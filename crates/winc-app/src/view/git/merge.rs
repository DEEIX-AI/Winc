//! 在文件里解决合并冲突的绘制：冲突各部分的底色、标记行上的标题和按钮、右上角的卡片。
//!
//! 当前这一侧铺蓝、对方铺绿、共同祖先铺灰，和差异视图用同一组柔和色。标记行不显示原来的
//! `<<<<<<<`，换成一条标题：`<<<<<<<` 那条写「当前 · HEAD」，后面是「采用当前」「采用对方」
//! 「两者都要」「对比」，右端是第几处；`|||||||` 写「共同祖先」，`=======` 写「对方 · 分支名」，
//! `>>>>>>>` 只是一道收尾的细线。

use winc_editor::git::merge::{Conflict, Pick};
use winc_i18n::t;

use crate::app::git::merge::MergeTarget;
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
    let (message, buttons): (String, Vec<MergeTarget>) = if count > 0 {
        (
            t!("git-merge-card-conflicts", count = count),
            vec![
                MergeTarget::Previous,
                MergeTarget::Next,
                MergeTarget::All(Pick::Ours),
                MergeTarget::All(Pick::Theirs),
            ],
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
            b => frame.measure(FontKind::Ui, font, all_label(b)) + 16.0,
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
                    all_label(target),
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

fn all_label(target: MergeTarget) -> &'static str {
    match target {
        MergeTarget::All(Pick::Ours) => t!("git-merge-accept-all-current"),
        MergeTarget::All(Pick::Theirs) => t!("git-merge-accept-all-incoming"),
        other => label(other),
    }
}
