//! 功能对话框（新建工作树、新建分支）的绘制。
//!
//! 淡遮罩上方是编辑区底色的卡片：标题行与关闭按钮，表单（标签、输入框、勾选框），右下角「取消」
//! 「创建」。输入框与按钮使用提交视图的样式（[`field_box`]）；名称不可用时外圈为错误色、「创建」
//! 置灰，原因显示在输入框右端（[`FormPlan`]）。`Tab` 焦点在名称以外的部件上时绘制强调色细线，
//! `git` 执行期间主按钮显示「正在创建…」，`git` 拒绝时原因以错误色显示在底部按钮左侧。

use std::f32::consts::{FRAC_PI_2, PI};

use winc_i18n::t;
use winc_render::{FontKind, Frame, Rect, Stroke};

use super::*;
use crate::app::git::form::{FormKind, FormPlan};
use crate::layout::{DialogLayout, DialogPart};

/// 绘制对话框：遮罩，再绘制卡片（淡入途中按 [`draw_moving_card`] 整张叠上去）。
pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let layout = app.dialog_layout();
    let p = app.palette;
    match app.dialog_card.motion {
        Some((true, clock)) => {
            let fade = dialog_curve(true, clock).0;
            frame.rect(app.layout.window, p.scrim.alpha(p.scrim.a * fade));
        }
        // 卡片盖住的地方不用再铺遮罩。
        _ => {
            let covered = frame.covered(layout.card, RADIUS_CARD);
            frame.rect_except(app.layout.window, p.scrim, 0.0, covered);
        }
    }
    let card = layout.card;
    draw_moving_card(app, frame, Card::Dialog, card, |app, frame| {
        draw_card_frame(frame, app.palette, card, RADIUS_CARD);
        draw_card(app, frame, &layout);
    });
}

/// 关闭后淡出期间绘制：遮罩和卡片一起淡出。
pub(super) fn draw_ghost(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    if let Some((clock, _, _)) = app.dialog_card.closing() {
        let fade = dialog_curve(false, clock).0;
        frame.rect(app.layout.window, p.scrim.alpha(p.scrim.a * fade));
    }
    draw_card_ghost(app, frame, Card::Dialog);
}

/// 绘制卡片内部：标题行、几行表单、选项和底部按钮。
fn draw_card(app: &mut App, frame: &mut Frame<'_>, layout: &DialogLayout) {
    let p = app.palette;
    let Some(dialog) = &app.form_dialog else {
        return;
    };
    let plan = app.form_plan(dialog);
    let hover = dialog.hover;
    let (copy, new_window, checkout) = (dialog.copy, dialog.new_window, dialog.checkout);
    let (running, error, focus) = (dialog.running.is_some(), dialog.error.clone(), dialog.focus);
    let empty = dialog.name.text().trim().is_empty();
    let (title, labels, placeholder) = match dialog.kind {
        FormKind::Worktree => (
            t!("common-dialog-new-worktree"),
            [
                t!("common-dialog-branch"),
                t!("common-dialog-base"),
                t!("common-dialog-location"),
            ],
            t!("common-dialog-worktree-placeholder"),
        ),
        FormKind::Branch { .. } => (
            t!("common-dialog-new-branch"),
            [t!("common-dialog-name"), t!("common-dialog-base"), ""],
            t!("common-dialog-branch-placeholder"),
        ),
    };
    let card = layout.card;
    frame.rounded(card, p.editor_bg, RADIUS_CARD);

    let header = layout.header;
    let x = header.x + SETTINGS_PAD;
    ui_text(frame, app, x, header, title, p.text, card);
    let close = layout.close;
    draw_button(
        frame,
        p,
        close,
        card,
        hover == Some(DialogPart::Close),
        false,
    );
    icons::close(frame, center(close), 7.0, p.text_muted, close);

    // 标签在名称列里右对齐，紧挨着输入框。
    for (rect, text) in layout.labels.iter().zip(labels) {
        let w = frame.measure(FontKind::Ui, app.fonts.ui, text);
        ui_text(
            frame,
            app,
            rect.right() - w,
            *rect,
            text,
            p.text_muted,
            card,
        );
    }

    draw_branch(app, frame, layout, &plan, placeholder);

    // 基于：能选时右端一对上下小箭头，悬停铺浅色；签出已有的分支时没有起点可选，字是淡的。
    let base = layout.base;
    field_box(app, frame, base, p.editor_bg, false, false);
    let area = base.inset(DIALOG_FIELD_INSET, 0.0);
    let color = if plan.base_enabled {
        if hover == Some(DialogPart::Base) {
            frame.rounded(base, p.hover, RADIUS_ROW);
        }
        let arrows = (area.right() - 3.0, base.y + base.h / 2.0);
        icons::up_down(frame, arrows, p.text_dim, card);
        p.text
    } else {
        p.text_dim
    };
    let text = truncate_end(frame, &plan.base, app.fonts.ui, (area.w - 20.0).max(0.0));
    ui_text(frame, app, area.x, base, &text, color, card);

    if let Some((rect, change)) = layout.location {
        draw_location(app, frame, layout, rect, change, &plan, empty);
    }

    for &(part, row) in &layout.options {
        let (text, on) = match part {
            DialogPart::Copy => (t!("common-dialog-copy-ignored"), copy),
            DialogPart::NewWindow => (t!("common-dialog-open-in-new-window"), new_window),
            _ => (t!("common-dialog-checkout-after-create"), checkout),
        };
        let end = draw_check_option(app, frame, row, text, on);
        if part == DialogPart::Copy {
            // 名称后面淡淡地跟着要复制的（配置 `git.worktreeCopy`）。
            let globs = app
                .config
                .git_worktree_copy
                .join(t!("common-list-separator"));
            let room = (row.right() - end - 8.0).max(0.0);
            let globs = ellipsize(frame, &globs, app.fonts.ui_small, room);
            ui_text_small(frame, app, end + 8.0, row, &globs, p.text_dim, card);
        }
    }

    // 按下「创建」后 git 仍在执行时，主按钮显示「正在创建…」且不可再按；完成后才关闭对话框。
    draw_action(
        app,
        frame,
        layout.cancel,
        t!("common-cancel"),
        false,
        true,
        hover,
    );
    let (confirm, enabled) = match running {
        true => (t!("common-dialog-creating"), false),
        false => (plan.confirm, plan.enabled),
    };
    draw_action(app, frame, layout.confirm, confirm, true, enabled, hover);
    if let Some(error) = error {
        draw_error(app, frame, layout, &error);
    }

    // 键盘焦点（经 `Tab` 移入）在名称以外的部件上时，部件外绘制一圈强调色细线；
    // 名称框有焦点时由输入框自行绘制外圈。
    let ring = match focus {
        DialogPart::Branch | DialogPart::Close => None,
        DialogPart::Base => Some(layout.base),
        DialogPart::Change => layout.location.map(|(_, change)| change),
        DialogPart::Cancel => Some(layout.cancel),
        DialogPart::Confirm => Some(layout.confirm),
        part => (layout.options.iter())
            .find(|(p, _)| *p == part)
            .map(|&(_, row)| Rect::new(row.x - 1.0, row.y + row.h / 2.0 - 7.0, 14.0, 14.0)),
    };
    if let Some(rect) = ring {
        focus_outline(
            frame,
            rect.inset(-2.0, -2.0),
            RADIUS_ROW + 2.0,
            p.accent,
            card,
        );
    }
}

/// 沿 `rect` 绘制圆角半径为 `r` 的细线，用于表示键盘焦点。
fn focus_outline(frame: &mut Frame<'_>, rect: Rect, r: f32, color: Color, clip: Rect) {
    let r = r.min(rect.w / 2.0).min(rect.h / 2.0);
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
    let edges = [
        Stroke::Line(x0 + r, y0, x1 - r, y0),
        Stroke::Arc(x1 - r, y0 + r, r, -FRAC_PI_2, 0.0),
        Stroke::Line(x1, y0 + r, x1, y1 - r),
        Stroke::Arc(x1 - r, y1 - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(x1 - r, y1, x0 + r, y1),
        Stroke::Arc(x0 + r, y1 - r, r, FRAC_PI_2, PI),
        Stroke::Line(x0, y1 - r, x0, y0 + r),
        Stroke::Arc(x0 + r, y0 + r, r, -PI, -FRAC_PI_2),
    ];
    frame.stroke(&edges, 1.5, color, clip);
}

/// 以错误色小字在底部按钮左侧绘制创建失败的原因，最多两行，超出部分截短。
fn draw_error(app: &App, frame: &mut Frame<'_>, layout: &DialogLayout, error: &str) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let left = layout.card.x + SETTINGS_PAD;
    let room = (layout.cancel.x - 16.0 - left).max(0.0);
    // 逐字符测量宽度，放不下时换行；第二行仍放不下时截短。
    let text = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lines = vec![String::new()];
    let mut width = 0.0;
    for c in text.chars() {
        let w = frame.measure(FontKind::Ui, size, c.encode_utf8(&mut [0; 4]));
        if width + w > room && lines.len() < 2 {
            lines.push(String::new());
            width = 0.0;
        }
        width += w;
        if let Some(line) = lines.last_mut() {
            line.push(c);
        }
    }
    let last = lines.len() - 1;
    lines[last] = ellipsize(frame, &lines[last], size, room).into_owned();
    let h = 16.0;
    let top = layout.cancel.y + (layout.cancel.h - h * lines.len() as f32) / 2.0;
    for (i, line) in lines.iter().enumerate() {
        let row = Rect::new(left, top + i as f32 * h, room, h);
        ui_text_small(frame, app, left, row, line, p.error, layout.card);
    }
}

/// 绘制分支名的输入框：有焦点时与设置中正在编辑的输入框相同，编辑区底色、外圈强调色。
///
/// 名字不可用时外圈是错误色，原因用错误色小字写在框的右端（最多占框的六成，输入的字在它左边）。
fn draw_branch(
    app: &mut App,
    frame: &mut Frame<'_>,
    layout: &DialogLayout,
    plan: &FormPlan,
    placeholder: &str,
) {
    let p = app.palette;
    let rect = layout.branch;
    let focused = app.focus == crate::app::Focus::Dialog
        && app
            .form_dialog
            .as_ref()
            .is_some_and(|d| d.focus == DialogPart::Branch);
    field_box(
        app,
        frame,
        rect,
        p.editor_bg,
        focused,
        focused && plan.problem.is_some(),
    );
    let Some(mut dialog) = app.form_dialog.take() else {
        return;
    };
    let mut area = rect.inset(DIALOG_FIELD_INSET, 0.0);
    if let Some(problem) = &plan.problem {
        let size = app.fonts.ui_small;
        let text = ellipsize(frame, problem, size, (area.w * 0.6).round());
        let w = frame.measure(FontKind::Ui, size, &text);
        ui_text_small(frame, app, area.right() - w, rect, &text, p.error, rect);
        area.w = (area.w - w - 12.0).max(0.0);
    }
    let caret = draw_bar_field(app, frame, area, &mut dialog.name, placeholder, focused);
    app.form_dialog = Some(dialog);
    if caret.is_some() {
        app.ime_cursor = caret;
    }
}

/// 绘制「位置」：目录从中间截短（开头在哪、最后的分支名都保留），还没输入名字时最后一级淡淡地
/// 写「分支名称」；右端一个目录图标（选择其他目录），与「基于」右端的上下箭头颜色相同，悬停时
/// 铺浅色。
fn draw_location(
    app: &App,
    frame: &mut Frame<'_>,
    layout: &DialogLayout,
    rect: Rect,
    change: Rect,
    plan: &FormPlan,
    empty: bool,
) {
    let p = app.palette;
    field_box(app, frame, rect, p.editor_bg, false, false);
    let x = rect.x + DIALOG_FIELD_INSET;
    let area = Rect::new(x, rect.y, (change.x - 8.0 - x).max(0.0), rect.h);
    let size = app.fonts.ui;
    match &plan.dir {
        Some(dir) if empty => {
            let parent = dir.parent().unwrap_or(dir);
            let head = format!("{}/", winc_editor::display_path(parent, None));
            let tail = t!("common-dialog-branch-name");
            let room = area.w - frame.measure(FontKind::Ui, size, tail);
            let head = ellipsize(frame, &head, size, room.max(0.0));
            let end = ui_text(frame, app, area.x, rect, &head, p.text_muted, area);
            ui_text(frame, app, end, rect, tail, p.text_dim, area);
        }
        Some(dir) => {
            let text = winc_editor::display_path(dir, None);
            let text = ellipsize(frame, &text, size, area.w);
            ui_text(frame, app, area.x, rect, &text, p.text_muted, area);
        }
        None => {
            ui_text(frame, app, area.x, rect, "—", p.text_dim, area);
        }
    }

    let hovered = app
        .form_dialog
        .as_ref()
        .is_some_and(|d| d.hover == Some(DialogPart::Change));
    if hovered {
        frame.rounded(change, p.hover, RADIUS_ROW - 2.0);
    }
    let (cx, cy) = center(change);
    let at = ((cx - 7.5).round(), (cy - 6.0).round());
    icons::folder(frame, at, false, p.text_dim, layout.card);
}

/// 绘制底部的一个按钮（见 [`draw_action_button`]）。
fn draw_action(
    app: &App,
    frame: &mut Frame<'_>,
    rect: Rect,
    text: &str,
    primary: bool,
    enabled: bool,
    hover: Option<DialogPart>,
) {
    let part = if primary {
        DialogPart::Confirm
    } else {
        DialogPart::Cancel
    };
    let role = if primary {
        ButtonRole::Primary
    } else {
        ButtonRole::Normal
    };
    let hovered = hover == Some(part);
    draw_action_button(app, frame, rect, text, role, enabled, hovered);
}
