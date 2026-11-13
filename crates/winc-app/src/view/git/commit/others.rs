//! 「显示全部仓库」时其他仓库各行的绘制：仓库的标题行和其下只读的文件行。

use super::*;
use crate::app::OtherRow;

/// 从上沿 `top`（逻辑像素）起绘制其他仓库的各行，记下可点击的位置。
///
/// 各行排在当前仓库的更改之后，随列表一起滚动；文件行与当前仓库的文件行同列对齐，没有勾选框。
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
            OtherRow::File(repo, entry) => {
                draw_other_file(app, frame, (list, band), (repo, entry), hits);
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

/// 画另一个仓库的标题行：文件夹图标、仓库名、所在的分支，右端是更改数。
fn draw_repository_row(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    repo: usize,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let target = CommitTarget::OtherRepository(repo);
    shade_hovered(app, frame, (list, band), target);
    let cy = band.y + COMMIT_ROW / 2.0;
    let state = (app.git.repositories.get(repo)).map(|root| app.place_state(root));
    let state = state.unwrap_or_default();
    // 名字和分支截到更改数之前，不与它重叠。
    let count = state.changes.to_string();
    let count_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &count) + 2.0 * END_CENTER;
    let clip = Rect::new(list.x, band.y, band.right() - count_w - list.x, COMMIT_ROW);
    let clip = clip.intersect(list);
    let icon_x = band.x + ARROW;
    icons::folder_small(frame, (icon_x, cy), p.text_muted, clip);
    let name = app.git_repository_name(repo);
    let mut x = ui_text_small(frame, app, icon_x + ICON, band, &name, p.text, clip);
    if let Some(head) = &state.head {
        x += BRANCH_LEAD;
        icons::branch_small(frame, (x, cy), p.text_dim, clip);
        ui_text_small(frame, app, x + BRANCH_TEXT, band, head, p.text_dim, clip);
    }
    draw_count_pill(app, frame, band, &count, list);
    hits.push((band.intersect(list), target));
}

/// 仓库名的末尾到分支图标中心的距离（逻辑像素）。
const BRANCH_LEAD: f32 = 14.0;
/// 分支图标的中心到分支名的距离（逻辑像素）：图标宽约 10，与文字相隔 3。
const BRANCH_TEXT: f32 = 8.0;

/// 画另一个仓库中的一个文件：图标、名字、淡色的所在目录，右端是状态字母。
fn draw_other_file(
    app: &App,
    frame: &mut Frame<'_>,
    (list, band): (Rect, Rect),
    (repo, entry): (usize, usize),
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let read = |root: &Path, status: &Status| {
        let entry = status.entries.get(entry)?;
        let name = entry.path.file_name()?.to_string_lossy().into_owned();
        let dir = entry.path.parent().and_then(|d| d.strip_prefix(root).ok());
        let dir = dir.map(|d| d.to_string_lossy().into_owned());
        Some((name, dir.unwrap_or_default(), entry.shown(), entry.is_dir))
    };
    let Some((name, dir, change, is_dir)) = app.other_status(repo, read).flatten() else {
        return;
    };
    let target = CommitTarget::OtherFile(repo, entry);
    shade_hovered(app, frame, (list, band), target);
    let cy = band.y + COMMIT_ROW / 2.0;
    // 与当前仓库平铺时的文件行同列：勾选框的位置空着，图标和名字对齐。
    let check = band.x + ARROW + file_column(app, 0) as f32 * INDENT + check_gap(app);
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
    ui_text_small(
        frame,
        app,
        letter_x,
        band,
        &letter,
        git_color(p, change),
        list,
    );
    let clip = Rect::new(list.x, band.y, letter_x - 8.0 - list.x, COMMIT_ROW).intersect(list);
    let end = ui_text(frame, app, text_x, band, &name, file_color(change, p), clip);
    if !dir.is_empty() {
        ui_text_small(frame, app, end + 8.0, band, &dir, p.text_dim, clip);
    }
    hits.push((band.intersect(list), target));
}
