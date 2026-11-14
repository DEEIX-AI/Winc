//! 「显示全部仓库」时其他仓库各行的绘制：仓库的标题行和其下的文件行。

use super::*;
use crate::app::OtherRow;

/// 从上沿 `top`（逻辑像素）起绘制其他仓库的各行，记下可点击的位置。
///
/// 各行排在当前仓库的更改之后，随列表一起滚动；文件行与当前仓库的文件行同列对齐。勾选式下
/// 各行带勾选框，勾上的文件随当前仓库的一起提交；暂存区视图中文件分为两组，行尾是暂存、
/// 取消暂存的按钮。
pub(super) fn draw_other_rows(
    app: &App,
    frame: &mut Frame<'_>,
    list: Rect,
    top: f32,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    for (i, row) in app.commit_other_rows().into_iter().enumerate() {
        let y = top + i as f32 * COMMIT_ROW;
        if y + COMMIT_ROW < list.y || y > list.bottom() {
            continue;
        }
        let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
        match row {
            OtherRow::Clean => {
                let (x, text) = (band.x + ARROW, t!("git-commit-button-nothing"));
                ui_text_small(frame, app, x, band, text, app.palette.text_dim, list);
            }
            OtherRow::Repository(repo) => draw_repository_row(app, frame, (list, band), repo, hits),
            OtherRow::Group(repo, staged) => {
                draw_other_group(app, frame, (list, band), (repo, staged), hits);
            }
            OtherRow::Staged(repo, entry) => {
                draw_other_file(app, frame, (list, band), (repo, entry, Some(true)), hits);
            }
            OtherRow::File(repo, entry) => {
                let side = app.git.commit.staging().then_some(false);
                draw_other_file(app, frame, (list, band), (repo, entry, side), hits);
            }
        }
    }
}

/// 鼠标停在 `target` 上时给这一行铺悬停的底色。
fn shade_hovered(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    target: CommitTarget,
) {
    if app.pointer.hover == Hover::Commit(target) {
        let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
        frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
    }
}

/// 画另一个仓库的标题行：勾选框（勾选式下）、文件夹图标、仓库名、所在的分支和与上游的差异，
/// 右端是更改数。
fn draw_repository_row(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    repo: usize,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let target = CommitTarget::OtherRepository(repo);
    // 每个仓库一节：标题行上沿一条分隔线，把它和上一个仓库的文件分开。
    let line = Rect::new(band.x, band.y, band.w, 1.0);
    frame.rect_clipped(line, p.separator, 0.0, list);
    shade_hovered(app, frame, (list, band), target);
    let cy = band.y + COMMIT_ROW / 2.0;
    let state = (app.git.repositories.get(repo)).map(|root| app.place_state(root));
    let state = state.unwrap_or_default();
    // 名字和分支截到更改数之前，不与它重叠。
    let count = state.changes.to_string();
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count) + 2.0 * END_CENTER;
    let clip = Rect::new(list.x, band.y, band.right() - count_w - list.x, COMMIT_ROW);
    let clip = clip.intersect(list);
    // 勾选式下标题行前面有勾选框，和当前仓库各组标题的勾选框同列：勾上或取消这个仓库的全部
    // 文件。暂存区视图中没有。
    let mut icon_x = band.x + ARROW;
    let check = check_gap(app);
    if check > 0.0 {
        let at = (band.x + ARROW + check, cy);
        let state = app.other_group_state(repo);
        draw_other_checkbox(app, frame, at, state, list);
        let hit = Rect::new(at.0 - 8.0, band.y + 4.0, 16.0, 16.0);
        hits.push((hit.intersect(list), CommitTarget::OtherGroupCheck(repo)));
        icon_x = at.0 + ICON;
    }
    icons::folder_small(frame, (icon_x + 6.0, cy), p.text_muted, clip);
    let name = app.git_repository_name(repo);
    let name_x = icon_x + 6.0 + ICON;
    let mut x = ui_text_small(frame, app, name_x, band, &name, p.text, clip);
    if let Some(head) = &state.head {
        // 分支是一个可以点的小块：点了切换到这个仓库并展开它的分支选择面板。
        let branch = CommitTarget::OtherBranch(repo);
        let on = app.pointer.hover == Hover::Commit(branch);
        let ink = if on { p.text } else { p.text_dim };
        let left = x + BRANCH_LEAD - 8.0;
        let size = app.fonts.ui_small;
        let end = x + BRANCH_LEAD + BRANCH_TEXT + frame.measure(FontKind::Ui, size, head);
        let chip = Rect::new(left, band.y + 3.0, end + 6.0 - left, band.h - 6.0);
        draw_button(frame, p, chip, clip, on, false);
        x += BRANCH_LEAD;
        icons::branch_small(frame, (x, cy), ink, clip);
        x = ui_text_small(frame, app, x + BRANCH_TEXT, band, head, ink, clip);
        hits.push((chip.intersect(clip), branch));
    }
    // 与上游的差异跟在分支名后面：有提交可推送或拉取时用强调色。
    let sync = match (state.behind, state.ahead) {
        (0, 0) => String::new(),
        (0, ahead) => format!("↑{ahead}"),
        (behind, 0) => format!("↓{behind}"),
        (behind, ahead) => format!("↓{behind} ↑{ahead}"),
    };
    if !sync.is_empty() {
        ui_text_small(frame, app, x + 8.0, band, &sync, p.accent, clip);
    }
    draw_count_pill(app, frame, band, &count, list);
    // 勾选框的点击区域先记，整行的后记：点在勾选框上只勾选，不切换仓库。
    hits.push((band.intersect(list), target));
}

/// 在 `at` 画其他仓库各行的勾选框，样式与当前仓库各行的相同。
fn draw_other_checkbox(
    app: &App,
    frame: &mut Frame<'_>,
    at: (f32, f32),
    state: Option<bool>,
    list: Rect,
) {
    let p = app.palette;
    let border = p.text_dim.alpha(0.55);
    icons::checkbox(frame, at, state, p.accent, border, p.text_on_accent, list);
}

/// 仓库名的末尾到分支图标中心的距离（逻辑像素）。
const BRANCH_LEAD: f32 = 14.0;
/// 分支图标的中心到分支名的距离（逻辑像素）：图标宽约 10，与文字相隔 3。
const BRANCH_TEXT: f32 = 8.0;

/// 画行尾的暂存（`on` 为真，加号）或取消暂存（减号）按钮，位置与当前仓库各行悬停时的按钮相同；
/// 返回它的左边缘。
fn draw_stage_button(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    target: CommitTarget,
    on: bool,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> f32 {
    let p = app.palette;
    let rect = file_actions(band).1;
    let hovered = app.pointer.hover == Hover::Commit(target);
    draw_button(frame, p, rect, list, hovered, false);
    let color = if hovered { p.text } else { p.text_muted };
    match on {
        true => icons::plus(frame, center(rect), 8.0, color, list),
        false => icons::minus(frame, center(rect), 8.0, color, list),
    }
    hits.push((rect.intersect(list), target));
    rect.x
}

/// 画暂存区视图中另一个仓库的分组行：组名，右端是文件数；悬停时文件数换成暂存或取消暂存整组
/// 的按钮。
fn draw_other_group(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    (repo, staged): (usize, bool),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let count = |status: &Status| {
        let entries = status.entries.iter();
        let ready = |e: &&winc_editor::git::Entry| e.staged.is_some() && !e.conflicted();
        match staged {
            true => entries.filter(ready).count(),
            false => entries
                .filter(|e| e.unstaged.is_some() || !ready(e))
                .count(),
        }
    };
    let count = app.other_status(repo, |_, status| count(status));
    let title = match staged {
        true => t!("git-commit-group-staged"),
        false => t!("git-commit-group-changes"),
    };
    let (row, stage) = (
        CommitTarget::OtherGroup(repo, staged),
        CommitTarget::OtherGroupStage(repo, !staged),
    );
    let hover = app.pointer.hover;
    let hovered = hover == Hover::Commit(row) || hover == Hover::Commit(stage);
    shade_hovered(app, frame, (list, band), row);
    if hovered {
        draw_stage_button(app, frame, (list, band), stage, !staged, hits);
    } else {
        draw_count_pill(app, frame, band, &count.unwrap_or(0).to_string(), list);
    }
    // 组名与其下文件行的图标同列。
    let x = band.x + ARROW + file_column(app, 0) as f32 * INDENT + ICON;
    ui_text_small(frame, app, x, band, title, p.text_muted, list);
    hits.push((band.intersect(list), row));
}

/// 画另一个仓库中的一个文件：图标、名字、淡色的所在目录，右端是状态字母。
///
/// `side` 是暂存区视图中这一行所在的部分：`Some(true)` 为已暂存，`Some(false)` 为未暂存；
/// 此时行尾是取消暂存或暂存的按钮，状态字母在按钮左边。勾选式下为 `None`。
fn draw_other_file(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    (repo, entry, side): (usize, usize, Option<bool>),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let read = |root: &Path, status: &Status| {
        let entry = status.entries.get(entry)?;
        let name = entry.path.file_name()?.to_string_lossy().into_owned();
        let dir = entry.path.parent().and_then(|d| d.strip_prefix(root).ok());
        let dir = dir.map(|d| d.to_string_lossy().into_owned());
        let change = match side {
            Some(true) => entry.staged.unwrap_or(entry.shown()),
            Some(false) => entry.unstaged.unwrap_or(entry.shown()),
            None => entry.shown(),
        };
        let fixed = entry.conflicted();
        Some((name, dir.unwrap_or_default(), change, entry.is_dir, fixed))
    };
    let Some((name, dir, change, is_dir, fixed)) = app.other_status(repo, read).flatten() else {
        return;
    };
    let target = match side {
        Some(true) => CommitTarget::OtherStagedFile(repo, entry),
        _ => CommitTarget::OtherFile(repo, entry),
    };
    shade_hovered(app, frame, (list, band), target);
    // 悬停时右端的状态字母换成暂存或取消暂存的按钮。冲突中的文件在这里不能暂存：切换到该
    // 仓库解决。
    let stage = side
        .filter(|_| !fixed)
        .map(|staged| (CommitTarget::OtherStage(repo, entry, !staged), !staged));
    let hover = app.pointer.hover;
    let stage =
        stage.filter(|(stage, _)| hover == Hover::Commit(target) || hover == Hover::Commit(*stage));
    let button = stage.map(|(stage, on)| {
        // 悬停在按钮上时整行同样铺底色。
        if hover == Hover::Commit(stage) {
            let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
            frame.rect_clipped(shade, app.palette.hover, RADIUS_ROW, list);
        }
        draw_stage_button(app, frame, (list, band), stage, on, hits)
    });
    let cy = band.y + COMMIT_ROW / 2.0;
    // 与当前仓库平铺时的文件行同列：勾选式下前面是勾选框，暂存区视图中那一列没有。
    let check = band.x + ARROW + file_column(app, 0) as f32 * INDENT + check_gap(app);
    if check_gap(app) > 0.0 {
        let on = app.other_status(repo, |_, status| {
            (status.entries.get(entry)).is_some_and(|e| app.other_checked(&e.path))
        });
        let state = on.unwrap_or(false).then_some(true);
        draw_other_checkbox(app, frame, (check, cy), state, list);
        let hit = Rect::new(check - 8.0, band.y + 4.0, 16.0, 16.0);
        hits.push((hit.intersect(list), CommitTarget::OtherCheck(repo, entry)));
    }
    let icon_x = check + ICON;
    let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
    if is_dir {
        icons::folder(frame, (icon_x, (cy - 6.0).round()), false, p.folder, list);
    } else {
        let inset = (icons::FOLDER_WIDTH - icons::FILE_WIDTH) / 2.0;
        draw_file_icon(frame, p, (icon_x + inset, cy), &name, list);
    }
    let letter = change.letter().to_string();
    let letter_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &letter);
    let letter_x = band.right() - END_CENTER - letter_w / 2.0;
    if button.is_none() {
        let color = git_color(p, change);
        ui_text_small(frame, app, letter_x, band, &letter, color, list);
    }
    let text_end = button.map_or(letter_x - 8.0, |x| x - 4.0);
    let clip = Rect::new(list.x, band.y, text_end - list.x, COMMIT_ROW).intersect(list);
    let end = ui_text(frame, app, text_x, band, &name, file_color(change, p), clip);
    if !dir.is_empty() {
        ui_text_small(frame, app, end + 8.0, band, &dir, p.text_dim, clip);
    }
    hits.push((band.intersect(list), target));
}
