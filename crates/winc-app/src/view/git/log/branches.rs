//! 日志左侧的分支面板：「全部分支」，本地、远程各一组分支，正在看的那一个铺选中的底色。

use super::*;
use crate::app::git::log::{BRANCH_ROW, BRANCHES_W, BranchRow};

/// 绘制标题栏最左边显示、隐藏分支面板的按钮：面板显示着时按下的样子。
pub(super) fn draw_branches_button(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &LogParts,
    hits: &mut Vec<(Rect, LogTarget)>,
) {
    let p = app.palette;
    let button = parts.branches_button();
    let hovered = app.pointer.hover == Hover::Log(LogTarget::BranchesToggle);
    // 按要切换到的状态画：关闭途中按钮已经弹起，打开途中已经按下。
    let on = app.state.log_branches.unwrap_or(true) && parts.branches.w > 0.0;
    draw_button(frame, p, button, parts.header, hovered, on);
    let ink = if hovered || on { p.text } else { p.text_muted };
    icons::sidebar(frame, center(button), on, ink, parts.header);
    hits.push((button, LogTarget::BranchesToggle));
}

/// 绘制分支面板，返回最大滚动距离（逻辑像素）；名字截短了的行记进 `long`。
///
/// 一行一个分支：分支图标和名字，当前分支后面打 ✓；正在按它筛选的一行铺选中的底色，没有按
/// 分支筛选时「全部分支」铺选中的底色。组的标题小号淡色，前面是折叠箭头。右边一道分隔线。
pub(super) fn draw_branches(
    app: &App,
    frame: &mut Frame<'_>,
    view: &LogView,
    parts: &LogParts,
    (hits, long): (&mut Vec<(Rect, LogTarget)>, &mut Vec<usize>),
) -> f32 {
    let area = parts.branches;
    if area.w <= 0.0 {
        return 0.0;
    }
    let p = app.palette;
    frame.rect(
        Rect::new(area.right() - 1.0, area.y, 1.0, area.h),
        p.separator,
    );
    let area = Rect::new(area.x, area.y, area.w - 1.0, area.h);
    // 打开、关闭途中面板只露出右边一段：各行按完整的宽度排，跟着右边缘从左边滑进、滑出。
    let slid = Rect::new(
        area.right() - (BRANCHES_W - 1.0),
        area.y,
        BRANCHES_W - 1.0,
        area.h,
    );
    let rows = app.log_branch_rows();
    let filter = view.query.branch.as_deref();
    let top = area.y + LOG_LIST_PAD - app.git.log.branches_scroll;
    for (i, row) in rows.iter().enumerate() {
        let y = top + i as f32 * BRANCH_ROW;
        let band = Rect::new(slid.x + 6.0, y, slid.w - 12.0, BRANCH_ROW);
        if band.bottom() < area.y || band.y > area.bottom() {
            continue;
        }
        let target = LogTarget::BranchRow(i);
        let hovered = app.pointer.hover == Hover::Log(target);
        let chosen = match row {
            BranchRow::All => filter.is_none(),
            BranchRow::Branch { name, .. } => filter == Some(name.as_str()),
            BranchRow::Heading(_) => false,
        };
        if let Some(color) = row_color(p, chosen, false, hovered) {
            frame.rect_clipped(band, color, RADIUS_ROW, area);
        }
        let cy = y + BRANCH_ROW / 2.0;
        let x = band.x + 8.0;
        match row {
            BranchRow::All => {
                ui_text_small(
                    frame,
                    app,
                    x,
                    band,
                    t!("git-log-all-branches"),
                    p.text,
                    area,
                );
            }
            BranchRow::Heading(group) => {
                let open = !app.git.log.branch_folds.contains(group);
                let turn = if open { FRAC_PI_2 } else { 0.0 };
                icons::chevron_turned(frame, (x + 3.0, cy), 6.0, turn, p.text_dim, area);
                // 只有一个远程时标题写「远程分支」，几个远程时写远程名。
                let remotes = rows
                    .iter()
                    .filter(|r| matches!(r, BranchRow::Heading(Some(_))));
                let title = match group {
                    None => t!("menu-git-local-branches"),
                    Some(_) if remotes.count() == 1 => t!("menu-git-remote-branches"),
                    Some(remote) => remote.as_str(),
                };
                ui_text_small(frame, app, x + 14.0, band, title, p.text_dim, area);
            }
            BranchRow::Branch {
                label: name,
                current,
                ..
            } => {
                let x = x + crate::theme::TREE_INDENT;
                icons::branch_small(frame, (x + 4.0, cy), p.text_muted, area);
                let check = if *current { 16.0 } else { 0.0 };
                let room = (band.right() - x - 14.0 - check - 6.0).max(0.0);
                let shown = truncate_end(frame, name, app.fonts.ui_small, room);
                // 截短了：悬停时提示全名。
                if shown != name.as_str() {
                    long.push(i);
                }
                let end = ui_text_small(frame, app, x + 14.0, band, &shown, p.text, area);
                if *current {
                    icons::check(frame, (end + 10.0, cy), 9.0, p.accent, area);
                }
            }
        }
        hits.push((band.intersect(area), target));
    }
    let content = LOG_LIST_PAD + rows.len() as f32 * BRANCH_ROW + LOG_LIST_PAD;
    (content - area.h).max(0.0)
}
