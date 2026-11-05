//! 侧栏的提交视图：标题行与工具按钮、说明条（合并、变基暂停时）、多行提交信息、
//! 「提交 ⌄」拆分按钮，以及分组并按目录排成树的改动列表。
//!
//! 每个文件占一行：勾选框、文件图标、文件名（平铺时其后以淡色显示所在目录）、右端着色的
//! 状态字母；悬停时状态字母替换为「打开文件」「回滚」两个按钮。改动行不显示行数。
//! 坐标均为逻辑像素。

use std::f32::consts::FRAC_PI_2;
use std::path::Path;

use winc_editor::git::{Change, Operation, Status};
use winc_i18n::t;

use crate::app::git::log::display_local;
use crate::app::{
    COMMIT_ROW, CommitOverflow, CommitParts, CommitRow, CommitRows, CommitTarget, Focus,
    MESSAGE_LINE, MESSAGE_PAD,
};
use crate::icons::Direction;
use crate::view::workspace::sidebar::{
    CHAIN_FOLD, CHAIN_GAP, CHAIN_SEPARATOR, fold_chain, git_color,
};
use crate::view::*;

/// 绘制提交视图，返回提交信息里光标的矩形（窗口坐标，逻辑像素），供输入法定位候选框。
pub(in crate::view) fn draw_commit(app: &mut App, frame: &mut Frame<'_>) -> Option<Rect> {
    // 提交信息框随内容长高：先按框的宽度折好行（框的宽度和高度无关），框的高度按行数定，
    // 绘制时继续使用折好的行。
    let width = message_area(app, app.commit_parts().message).w;
    let focused = app.focus == Focus::CommitMessage;
    let mut field = std::mem::take(&mut app.commit.message);
    let wrap = wrap_area_field(app, frame, width, &mut field, focused);
    app.commit.message = field;
    app.set_commit_message_lines(wrap.lines());
    let parts = app.commit_parts();
    let mut hits = Vec::new();
    draw_header(app, frame, &parts, &mut hits);
    match (&app.git.status, &app.git.problem) {
        (Some(status), _) if !status.entries.is_empty() => {
            draw_list(app, frame, &parts, status, &mut hits);
        }
        (Some(_), _) => draw_empty(app, frame, &parts),
        (None, Some(problem)) => {
            let text = t!("git-commit-view-problem", problem = problem.as_str());
            draw_note(app, frame, &parts, &text, true);
        }
        (None, None) => draw_note(app, frame, &parts, t!("git-commit-view-loading"), false),
    }
    // 列表的滚动条贴在列表右边，盖在固定的行上面；放得下时没有。
    let content = app.commit_max_scroll() + parts.list.h;
    let scroll = app.commit.scroll;
    draw_scrollbar(
        app,
        frame,
        ScrollArea::Commit,
        0,
        parts.list,
        scroll,
        content,
    );
    draw_banner(app, frame, &parts, &mut hits);
    draw_undo(app, frame, &parts, &mut hits);
    let caret = draw_compose(app, frame, &parts, wrap, &mut hits);
    app.commit_hits = hits;
    app.commit.hits_generation = app.git.generation;
    caret
}

/// 绘制第二行右端的三个按钮（左边是共用的项目和分支）：「Git 日志」、树形和列表的切换、「⋯」。
///
/// 切换按钮画的是点下去以后的样子：平铺时画树形，排成树时画列表。「⋯」的下拉菜单里有刷新、
/// 全部展开 / 折叠、回滚勾选的文件。
fn draw_header(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let header = parts.header;
    let p = app.palette;
    for (rect, target) in parts.tools {
        let open = target == CommitTarget::More
            && app.menu_button == Some(crate::app::MenuButton::CommitMore);
        let hovered = app.hover == Hover::Commit(target);
        draw_button(frame, p, rect, header, hovered, open);
        let color = if hovered || open {
            p.text
        } else {
            p.text_muted
        };
        let c = center(rect);
        match target {
            CommitTarget::Log => icons::history(frame, c, color, header),
            CommitTarget::Layout if app.commit.flat() => icons::tree(frame, c, color, header),
            CommitTarget::Layout => icons::list(frame, c, color, header),
            _ => icons::more(frame, c, color, header),
        }
        hits.push((rect, target));
    }
}

/// 绘制合并、变基等暂停时提交信息上方的说明条。
///
/// 说明条显示进行中的操作和仍有冲突的文件数，右端为「中止」。仍有冲突时底色和图标使用冲突色；
/// 冲突全部解决后改用淡强调色，提示用户可以提交。
fn draw_banner(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let (Some((rect, abort)), Some(status)) = (parts.banner, app.git.status.as_ref()) else {
        return;
    };
    let Some(operation) = status.operation else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(rect);
    let unresolved = status.unresolved.len();
    let conflicted = status
        .entries
        .iter()
        .any(winc_editor::git::Entry::conflicted);
    let tint = if unresolved > 0 {
        p.git_conflict
    } else {
        p.accent
    };
    frame.rounded(rect, tint.alpha(0.12), RADIUS_ROW);
    let op = operation.label();
    let text = match (unresolved, conflicted) {
        (0, true) => t!("git-operation-banner-resolved", operation = op),
        (0, false) => t!("git-operation-banner", operation = op),
        (n, _) => t!("git-operation-banner-conflicts", operation = op, count = n),
    };
    let size = app.fonts.ui_small;
    let mut x = rect.x + 8.0;
    if unresolved > 0 {
        icons::warning(frame, (x + 6.0, icon_center_y(app, rect, size)), tint, clip);
        x += 18.0;
    }
    let room = Rect::new(rect.x, rect.y, abort.x - 4.0 - rect.x, rect.h);
    ui_text_small(frame, app, x, rect, &text, p.text, clip.intersect(room));
    let hovered = app.hover == Hover::Commit(CommitTarget::Abort);
    draw_button(frame, p, abort, clip, hovered, false);
    let w = frame.measure(FontKind::Ui, size, t!("git-operation-abort"));
    let color = if hovered { p.text } else { p.text_muted };
    let text_x = (abort.x + (abort.w - w) / 2.0).round();
    ui_text_small(
        frame,
        app,
        text_x,
        abort,
        t!("git-operation-abort"),
        color,
        clip,
    );
    hits.push((abort, CommitTarget::Abort));
}

/// 视图底部的上一次提交：上沿一条分隔线，左边是提交信息的第一行，右边淡色写提交时间，
/// 最右端是「撤销」。
///
/// 只在上一次提交还没推送时出现（见 `App::commit_undoable`）。
fn draw_undo(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let (Some((bar, button)), Some(last)) = (parts.undo, app.commit_undoable()) else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(bar);
    frame.rect_clipped(Rect::new(bar.x, bar.y, bar.w, 1.0), p.separator, 0.0, clip);
    let row = Rect::new(
        parts.message.x,
        button.y,
        button.right() - parts.message.x,
        button.h,
    );
    let size = app.fonts.ui_small;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let time = display_local(&last.local, last.time, now);
    let time_w = frame.measure(FontKind::Ui, size, &time);
    let time_x = (button.x - 8.0 - time_w).max(row.x);
    ui_text_small(frame, app, time_x, row, &time, p.text_dim, clip);
    let room = Rect::new(row.x, row.y, (time_x - 8.0 - row.x).max(0.0), row.h);
    let subject = fit(frame, &last.subject, size, room.w - 2.0, false);
    let subject_clip = clip.intersect(room);
    ui_text_small(
        frame,
        app,
        row.x + 2.0,
        row,
        &subject,
        p.text_muted,
        subject_clip,
    );

    let hovered = app.hover == Hover::Commit(CommitTarget::Undo);
    draw_button(frame, p, button, clip, hovered, false);
    let label = t!("git-undo-commit");
    let w = frame.measure(FontKind::Ui, size, label);
    let color = if hovered { p.text } else { p.text_muted };
    let text_x = (button.x + (button.w - w) / 2.0).round();
    ui_text_small(frame, app, text_x, button, label, color, clip);
    hits.push((button, CommitTarget::Undo));
}

/// 没有改动时列表区的空状态：居中的「没有更改」，下面一行淡色说明。
fn draw_empty(app: &App, frame: &mut Frame<'_>, parts: &CommitParts) {
    draw_empty_state(
        app,
        frame,
        (parts.list, sidebar_span(app)),
        t!("git-commit-view-empty"),
        Some(t!("git-commit-view-empty-detail")),
    );
}

/// 列表的位置上写一行说明（没有改动、正在读取、出错的原因）。
///
/// 出错时用警告色，前面一个警告图标，放不下时折成几行。
fn draw_note(app: &App, frame: &mut Frame<'_>, parts: &CommitParts, text: &str, warn: bool) {
    let p = app.palette;
    let list = parts.list;
    let x = list.x + 16.0;
    let room = (list.right() - 16.0 - x - if warn { 18.0 } else { 0.0 }).max(40.0);
    let color = if warn { p.warning } else { p.text_dim };
    // 按宽度折行：逐个字符量，放不下就换行。
    let mut lines = vec![String::new()];
    let mut width = 0.0;
    for c in text.chars() {
        let w = frame.measure(FontKind::Ui, app.fonts.ui_small, c.encode_utf8(&mut [0; 4]));
        if width + w > room && !lines.last().is_some_and(String::is_empty) {
            lines.push(String::new());
            width = 0.0;
        }
        width += w;
        if let Some(line) = lines.last_mut() {
            line.push(c);
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let row = Rect::new(list.x, list.y + 4.0 + i as f32 * 20.0, list.w, 20.0);
        let mut text_x = x;
        if warn && i == 0 {
            icons::warning(frame, (x + 6.0, row.y + row.h / 2.0), p.warning, list);
            text_x += 18.0;
        } else if warn {
            text_x += 18.0;
        }
        label(
            frame,
            app,
            app.fonts.ui_small,
            (text_x, row),
            line,
            color,
            list,
        );
    }
}

/// 画列表各行时共用的状态：每一行都要读取，一帧只取一次。
struct ListContext<'a> {
    list: Rect,
    status: &'a Status,
    /// 仓库的根目录。
    root: &'a Path,
    /// 画的是哪一版列表行（展开收起的动画期间是动画中的那一版）。
    rows: &'a CommitRows,
    /// 是否有多个文件被选中：目录行下面的文件全部选中时，目录行才画成选中状态。
    ///
    /// 每行是否选中按 `App::commit_is_selected` 查集合：几千个改动、多选时，逐项线性查找
    /// 会在一帧里做上千万次比较。
    multi: bool,
}

/// 绘制改动列表：按目录排成树（或者平铺），滚动时所在的组、目录固定在顶部。
fn draw_list(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    status: &Status,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let list = parts.list;
    let root = app.git.root.as_deref().unwrap_or(Path::new(""));
    let multi = app.commit_multi_selected();

    // 组、目录正在展开或收起：按动画中的那一版绘制。子行的位置不动，只露出已经露出来的那一段，
    // 下面的行随之上下移动。被展开、收起的行及其上方各行位置不变，行号在动画前后相同，仍记录
    // 命中区域，悬停底色不因动画中断；其余各行在动画期间不记录。
    if let Some(motion) = &app.commit.motion {
        let cx = ListContext {
            list,
            status,
            root,
            rows: &motion.rows,
            multi,
        };
        let (block, hidden) = (motion.block(), motion.hidden());
        for v in 0..motion.rows.len() {
            let (rect, clip) =
                crate::layout::tree_row_slot(list, app.commit.scroll, v, Some(block), hidden);
            let clip = clip.intersect(list);
            if clip.h <= 0.0 || rect.bottom() < list.y || rect.y > list.bottom() {
                continue;
            }
            let mut row_hits = Vec::new();
            frame.within(clip, |frame| {
                draw_row(app, frame, &cx, v, rect.y, &mut row_hits);
            });
            if v <= motion.parent {
                hits.extend(row_hits);
            }
        }
        // 固定的行仍然盖在上面（展开收起的是它们下面的行），否则动画过程中会先消失再出现。
        let mut ignored = Vec::new();
        draw_sticky(app, frame, &cx, &mut ignored);
        return;
    }

    let rows = app.commit_rows();
    let cx = ListContext {
        list,
        status,
        root,
        rows: &rows,
        multi,
    };
    let first = (app.commit.scroll / COMMIT_ROW).floor() as usize;
    let visible = (list.h / COMMIT_ROW).ceil() as usize + 1;

    for i in (first..rows.len()).take(visible) {
        let y = list.y + i as f32 * COMMIT_ROW - app.commit.scroll;
        draw_row(app, frame, &cx, i, y, hits);
    }

    draw_sticky(app, frame, &cx, hits);
}

/// 固定在顶部的组、目录：盖住下面滚过去的行，下沿一道细线。
///
/// 点击时先滚到那一行再收起或展开，勾选框仍可点击；命中范围记在 `hits` 最前面，先于
/// 下面的行判定。被下一个组、目录往上推的那一行只画在它自己那一格里，从上一层固定的行
/// 下面滑走，不盖住上一层，否则滚动时上下两行会叠在一起。
fn draw_sticky(
    app: &App,
    frame: &mut Frame<'_>,
    cx: &ListContext<'_>,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let list = cx.list;
    let sticky = app.commit_sticky(cx.rows);
    let mut pinned = Vec::new();
    for s in &sticky {
        let slot = Rect::new(
            list.x,
            list.y + s.level as f32 * COMMIT_ROW,
            list.w,
            COMMIT_ROW,
        )
        .intersect(list);
        let mut row_hits = Vec::new();
        frame.within(slot, |frame| {
            frame.rect_clipped(s.rect, p.sidebar_bg, 0.0, list);
            if app.hover == Hover::Commit(CommitTarget::Sticky(s.row, s.level)) {
                let band = Rect::new(
                    list.x + 8.0,
                    s.rect.y + 1.0,
                    list.w - 16.0,
                    COMMIT_ROW - 2.0,
                );
                frame.rect_clipped(band, p.hover, RADIUS_ROW, list);
            }
            draw_row(app, frame, cx, s.row, s.rect.y, &mut row_hits);
        });
        for (rect, target) in row_hits {
            let target = match target {
                CommitTarget::Group(_) | CommitTarget::Dir(_) => {
                    CommitTarget::Sticky(s.row, s.level)
                }
                other => other,
            };
            let rect = rect.intersect(slot);
            if rect.h > 0.0 {
                pinned.push((rect, target));
            }
        }
    }
    if let Some(bottom) = sticky.iter().map(|s| s.rect.bottom()).reduce(f32::max) {
        let line = Rect::new(list.x, bottom - 1.0, list.w, 1.0);
        frame.rect_clipped(line, p.separator, 0.0, list);
        // 固定的各行下面被盖住的行点不到。
        let covered = Rect::new(list.x, list.y, list.w, bottom - list.y);
        hits.retain(|(rect, _)| !covered.contains(rect.x + 1.0, rect.y + rect.h / 2.0));
    }
    hits.splice(0..0, pinned);
}

/// 画改动列表里的第 `i` 行，上沿在 `y`（逻辑像素）；记下可点击的位置。
fn draw_row(
    app: &App,
    frame: &mut Frame<'_>,
    cx: &ListContext<'_>,
    i: usize,
    y: f32,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let p = app.palette;
    let ListContext {
        list,
        status,
        root,
        rows,
        multi,
    } = *cx;
    let row = &rows[i];
    let focused = app.focus == Focus::CommitList;
    let checkbox = |frame: &mut Frame<'_>, x: f32, cy: f32, state: Option<bool>| {
        let border = p.text_dim.alpha(0.55);
        icons::soft_checkbox(frame, (x, cy), state, p.accent, border, list);
    };
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    // 鼠标停着的这一行名字放不下：写全名，状态字母让开；伸出侧栏时底色铺到侧栏边缘、右边是
    // 直角，和伸出去的那一段（`draw_commit_overflow`）连成一条。
    let full = app
        .commit
        .overflow
        .filter(|o| o.row == i && (o.y - y).abs() < 0.5);
    let spill = full.and_then(|o| o.spill);
    // 底色上下各让出 1 逻辑像素：相邻两行都有底色（选中的和悬停的）时中间留一道缝。
    let shade = Rect::new(band.x, band.y + 1.0, band.w, band.h - 2.0);
    let fill = |frame: &mut Frame<'_>, color: Color| {
        if spill.is_some() {
            let r = RADIUS_ROW;
            let wide = Rect::new(shade.x, shade.y, list.right() - shade.x, shade.h);
            frame.rect_corners(wide, color, [r, 0.0, 0.0, r], list);
        } else {
            frame.rect_clipped(shade, color, RADIUS_ROW, list);
        }
    };
    let cy = y + COMMIT_ROW / 2.0;
    // 箭头、勾选框的列（勾选框按中心）：每深一层往右缩进 `INDENT`，目录的箭头在勾选框左边。
    let arrow_x = |level: usize| band.x + ARROW + level as f32 * INDENT;
    let check_x = |level: usize| arrow_x(level) + CHECK;
    let text_clip = Rect::new(list.x, y, band.right() - 6.0 - list.x, COMMIT_ROW).intersect(list);
    // 缩进参考线：每一层上级目录一条，位于该目录的箭头正下方，相邻行连成一条竖线，
    // 目录层级深时能看出每个文件属于哪个目录。
    if let CommitRow::Dir { depth, .. } | CommitRow::File { depth, .. } = row {
        for level in 1..=*depth {
            let x = arrow_x(level).round();
            frame.rect_clipped(Rect::new(x, y, 1.0, COMMIT_ROW), p.indent_guide, 0.0, list);
        }
    }

    match row {
        CommitRow::Group(group) => {
            let group = *group;
            if app.hover == Hover::Commit(CommitTarget::Group(group)) {
                frame.rect_clipped(shade, p.hover, RADIUS_ROW, list);
            }
            let open = !app.commit.group_collapsed(group);
            chevron(app, frame, (arrow_x(0), cy), (i, open), p.text_dim, list);
            let group_check = check_x(0);
            let members = rows.group(group);
            let checks = app.commit.checks();
            let state = rows.members_state(members, checks, || {
                app.commit.members_state(status, members)
            });
            checkbox(frame, group_check, cy, state);
            // 组标题：小一号的次要色字，右端一个写文件数的小圆角徽标。
            label(
                frame,
                app,
                app.fonts.ui_small,
                (group_check + ICON, band),
                group.title(),
                p.text_muted,
                list,
            );
            draw_count_pill(app, frame, band, &members.len().to_string(), list);
            let check = Rect::new(group_check - 8.0, y + 4.0, 16.0, 16.0);
            hits.push((check.intersect(list), CommitTarget::GroupCheck(group)));
            hits.push((band.intersect(list), CommitTarget::Group(group)));
        }
        CommitRow::Dir {
            group,
            dir,
            label: name,
            depth,
        } => {
            // 组标题已经写了文件数，展开后每个文件都看得见，每层再写一遍是重复信息。
            let level = depth + 1;
            let hovered = matches!(
                app.hover,
                Hover::Commit(CommitTarget::Dir(r) | CommitTarget::DirCheck(r)) if r == i
            );
            // 多选时下面的文件都选中了：画成选中状态。
            let members = rows.dir(*group, dir);
            // 选中的文件比目录下面的少时不可能都选中，不必逐个查。
            let is_selected = multi
                && !members.is_empty()
                && app.commit_marked_count() >= members.len()
                && members
                    .iter()
                    .all(|&m| app.commit_is_selected(&status.entries[m].path));
            if let Some(color) = row_color(p, is_selected, focused, hovered || full.is_some()) {
                fill(frame, color);
            }
            let open = !app.commit.dir_collapsed(*group, dir);
            chevron(
                app,
                frame,
                (arrow_x(level), cy),
                (i, open),
                p.text_dim,
                list,
            );
            let dir_check = check_x(level);
            let checks = app.commit.checks();
            let state = rows.members_state(members, checks, || {
                app.commit.members_state(status, members)
            });
            checkbox(frame, dir_check, cy, state);
            // 目录和文件的图标在同一列（宽 `FOLDER_WIDTH`）里居中，名字从同一处开始，
            // 两种行排得整齐。
            let icon_x = dir_check + ICON;
            icons::folder(frame, (icon_x, (cy - 6.0).round()), open, p.folder, list);
            let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
            if full.is_some() {
                draw_dir_label(app, frame, (text_x, band), name, None, list);
            } else {
                let room = (text_clip.right() - text_x).max(0.0);
                draw_dir_label(app, frame, (text_x, band), name, Some(room), text_clip);
            }
            if let Some(spill) = spill {
                hits.push((spill, CommitTarget::Dir(i)));
            }
            let check = Rect::new(dir_check - 8.0, y + 4.0, 16.0, 16.0);
            hits.push((check.intersect(list), CommitTarget::DirCheck(i)));
            hits.push((band.intersect(list), CommitTarget::Dir(i)));
        }
        CommitRow::File { index, depth } => {
            let index = *index;
            let entry = &status.entries[index];
            let is_selected = app.commit_is_selected(&entry.path);
            let hovered = matches!(
                app.hover,
                Hover::Commit(
                    CommitTarget::File(i)
                        | CommitTarget::Check(i)
                        | CommitTarget::OpenFile(i)
                        | CommitTarget::RevertFile(i)
                ) if i == index
            );
            // 悬停时右端的状态字母替换为「打开文件」「回滚」两个按钮；文件名放不下而显示全名时，
            // 按钮跟在全名之后（见 `commit_overflow`）。
            let actions = match full {
                Some(overflow) => overflow.actions,
                None => hovered.then(|| file_actions(band)),
            };
            if let Some(color) = row_color(p, is_selected, focused, hovered || full.is_some()) {
                fill(frame, color);
            }
            // 文件没有箭头：勾选框和同一层目录的勾选框对齐；平铺时没有层级，和组标题的勾选框
            // 对齐。
            let file_check = check_x(file_column(app, *depth));
            checkbox(frame, file_check, cy, app.commit.file_state(entry));

            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let icon_x = file_check + ICON;
            // 未跟踪的整个目录在 Git 的输出里只有一项，画目录图标。文件图标在图标那一列里居中。
            if entry.is_dir {
                icons::folder(frame, (icon_x, (cy - 6.0).round()), false, p.folder, list);
            } else {
                let inset = (icons::FOLDER_WIDTH - icons::FILE_WIDTH) / 2.0;
                draw_file_icon(frame, p, (icon_x + inset, cy), &name, list);
            }
            let text_x = icon_x + icons::FOLDER_WIDTH + LABEL_GAP;
            // 名字用正文色（删除的文件用次要色），状态是右端一个上色的字母（M、U、D 等）；
            // 整行名字上色后颜色过多，不易分辨。
            let change = entry.shown();
            let letter = change.letter().to_string();
            let letter_w = frame.measure(FontKind::Ui, app.fonts.ui_small, &letter);
            let letter_x = band.right() - 10.0 - letter_w;
            // 文件名及平铺时的目录绘制到 `text_end` 为止。
            let text_end = actions.map_or(letter_x - 8.0, |(open, _)| open.x - 4.0);
            if let Some(actions) = actions {
                draw_file_actions(app, frame, actions, index, list);
                // 伸出侧栏的按钮同样可以点击，命中区域不裁到列表内。
                let (open, revert) = actions;
                for (rect, target) in [
                    (open, CommitTarget::OpenFile(index)),
                    (revert, CommitTarget::RevertFile(index)),
                ] {
                    let rect = if full.is_some() {
                        rect
                    } else {
                        rect.intersect(list)
                    };
                    hits.push((rect, target));
                }
            } else if full.is_none() {
                ui_text_small(
                    frame,
                    app,
                    letter_x,
                    band,
                    &letter,
                    git_color(p, change),
                    list,
                );
            }
            let color = file_color(change, p);
            let room = (text_end - text_x).max(0.0);
            let name_clip =
                Rect::new(list.x, y, (text_end - list.x).max(0.0), COMMIT_ROW).intersect(list);
            let name = if full.is_some() {
                name.into()
            } else {
                ellipsize(frame, &name, app.fonts.ui, room)
            };
            let end = ui_text(frame, app, text_x, band, &name, color, name_clip);
            // 平铺时名字后面淡色写所在目录（放不下从前面截掉，留下最近的几级）。
            if app.commit.flat()
                && let Some(parent) = entry
                    .path
                    .parent()
                    .and_then(|p| p.strip_prefix(root).ok())
                    .filter(|p| !p.as_os_str().is_empty())
            {
                let dir = parent.to_string_lossy();
                let room = (text_end - end - 8.0).max(0.0);
                let dir = if full.is_some() {
                    dir
                } else {
                    ellipsize(frame, &dir, app.fonts.ui_small, room)
                };
                ui_text_small(frame, app, end + 8.0, band, &dir, p.text_dim, name_clip);
            }
            if let Some(spill) = spill {
                hits.push((spill, CommitTarget::File(index)));
            }
            let check = Rect::new(file_check - 8.0, y + 4.0, 16.0, 16.0);
            hits.push((check.intersect(list), CommitTarget::Check(index)));
            hits.push((band.intersect(list), CommitTarget::File(index)));
        }
    }
}

/// 在 `row` 右端画一个写着 `count` 的小圆角徽标。
fn draw_count_pill(app: &App, frame: &mut Frame<'_>, row: Rect, count: &str, clip: Rect) {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let w = frame.measure(FontKind::Ui, size, count) + 12.0;
    let pill = Rect::new(row.right() - 4.0 - w, row.y + 5.0, w, row.h - 10.0);
    frame.rect_clipped(pill, p.hover, pill.h / 2.0, clip);
    caps_label(
        frame,
        app,
        size,
        (pill.x + 6.0, pill),
        count,
        p.text_muted,
        clip,
    );
}

/// 组、目录行（第 `i` 行）前面的箭头：展开时朝下，收起时朝右；这一行正在展开或收起时
/// 跟着动画转到对应角度。
fn chevron(
    app: &App,
    frame: &mut Frame<'_>,
    at: (f32, f32),
    (i, open): (usize, bool),
    color: Color,
    clip: Rect,
) {
    let turned = match &app.commit.motion {
        Some(m) if m.parent == i => 1.0 - m.hidden() / (m.count as f32 * COMMIT_ROW),
        _ => f32::from(u8::from(open)),
    };
    icons::chevron_turned(frame, at, 6.0, turned * FRAC_PI_2, color, clip);
}

/// 改动行悬停时右端按钮的边长（逻辑像素，经验值）。
const ACTION: f32 = 20.0;

/// 返回改动行 `band` 悬停时右端「打开文件」「回滚」两个按钮的位置。
fn file_actions(band: Rect) -> (Rect, Rect) {
    let y = band.y + (band.h - ACTION) / 2.0;
    let revert = Rect::new(band.right() - 4.0 - ACTION, y, ACTION, ACTION);
    let open = Rect::new(revert.x - 2.0 - ACTION, y, ACTION, ACTION);
    (open, revert)
}

/// 绘制第 `index` 个文件的「打开文件」「回滚」两个按钮（位置为 `open`、`revert`），只绘制在
/// `clip` 之内；悬停的按钮铺底色，图标用正文色。
fn draw_file_actions(
    app: &App,
    frame: &mut Frame<'_>,
    (open, revert): (Rect, Rect),
    index: usize,
    clip: Rect,
) {
    let p = app.palette;
    for (rect, target) in [
        (open, CommitTarget::OpenFile(index)),
        (revert, CommitTarget::RevertFile(index)),
    ] {
        let on = app.hover == Hover::Commit(target);
        draw_button(frame, p, rect, clip, on, false);
        let color = if on { p.text } else { p.text_muted };
        let (cx, cy) = center(rect);
        match target {
            CommitTarget::OpenFile(_) => {
                let (w, h) = (10.0, 12.0);
                let at = ((cx - w / 2.0).round(), (cy - h / 2.0).round());
                icons::document(frame, at, (w, h), color, clip);
            }
            _ => icons::rollback(frame, (cx, cy), color, clip),
        }
    }
}

/// 改动列表里一行的各列（逻辑像素）：箭头的中心离底色左边 `ARROW`，每深一层往右缩进
/// `INDENT`（与文件视图的树共用 `theme::TREE_INDENT`，16），图标和名字之间隔 `LABEL_GAP`。
///
/// 箭头和图标之间多一列勾选框：勾选框的中心在箭头右边 `CHECK`，图标从勾选框中心往右
/// `ICON` 开始。间距均为经验值。
const ARROW: f32 = 10.0;
const INDENT: f32 = crate::theme::TREE_INDENT;
const CHECK: f32 = 16.0;
const ICON: f32 = 12.0;
const LABEL_GAP: f32 = 5.0;

/// 返回提交信息框 `rect` 内放置文字行的区域。
///
/// 上下留 [`MESSAGE_PAD`]；行高中字形上下各有 `(MESSAGE_LINE - 字号) / 2` 的空白，左右的留白
/// 加上同样的宽度，使文字与边框的可见间距四边相等。
fn message_area(app: &App, rect: Rect) -> Rect {
    let side = MESSAGE_PAD + (MESSAGE_LINE - app.fonts.ui).max(0.0) / 2.0;
    rect.inset(side, MESSAGE_PAD)
}

/// 绘制提交信息（折好的行是 `wrap`）和「提交 ⌄」拆分按钮。
fn draw_compose(
    app: &mut App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    wrap: AreaWrap,
    hits: &mut Vec<(Rect, CommitTarget)>,
) -> Option<Rect> {
    let p = app.palette;
    let panel = app.layout.sidebar;

    // 提交信息框平时没有边框，铺一层比侧栏深一点的中性底色；有焦点时换成编辑区的底色，
    // 外圈画强调色。
    let focused = app.focus == Focus::CommitMessage;
    let rect = parts.message;
    field_box(app, frame, rect, p.sidebar_bg, focused, false);
    // 框高过渡途中文字按过渡结束后的高度排版，只绘制在当前的框内，行的位置和滚动量不随框高变化。
    let area = message_area(app, parts.message_settled);
    let mut field = std::mem::take(&mut app.commit.message);
    // 占位文字只写快捷键：分支就在上面一行，长分支名写进来会被截掉。
    let key = if cfg!(target_os = "macos") {
        "⌘↩"
    } else {
        "Ctrl+Enter"
    };
    let placeholder = if app.commit.amend {
        t!("git-commit-message-amend-placeholder").to_owned()
    } else {
        t!("git-commit-message-placeholder", key = key)
    };
    let caret = frame.within(rect, |frame| {
        draw_area_field(app, frame, area, &mut field, wrap, &placeholder, focused)
    });
    app.commit.message = field;

    // 「提交 ⌄」：一整条强调色的拆分按钮，左边「✓ 提交」，右边一道细竖线隔开下拉箭头。
    // 无法提交（见 `CommitState::blocker`：未勾选文件、未填写说明、仍有冲突标记）时按钮整体
    // 变淡且悬停不变色，表示按下不会生效；变基暂停时无需勾选即可继续。正在提交时保持强调色，
    // 只是不响应悬停，按钮不在提交前后闪成灰色。没有改动而与上游有差异时，按钮用于同步
    // （`App::commit_sync`）。
    let (main, menu) = (parts.commit, parts.commit_menu);
    let whole = Rect::new(main.x, main.y, menu.right() - main.x, main.h);
    let sync = app.commit_sync();
    let syncing = app.commit_syncing();
    let busy = app.commit.busy || syncing;
    let operation = app.git.status.as_ref().and_then(|s| s.operation);
    let ready = !busy
        && (sync.is_some()
            || app
                .git
                .status
                .as_ref()
                .is_some_and(|s| app.commit.blocker(s).is_none()));
    // 刚提交成功时提交信息已清空，按钮本应变淡；显示「已提交」期间仍使用强调色。
    let done = app.commit.committed.as_ref().filter(|_| !busy);
    // 不能提交时用灰底、淡色字，表示不可用；把蓝底和白字一起调淡时，浅色主题里的对比度不足。
    let (fill, ink, shade) = if ready || busy || done.is_some() {
        (
            p.accent_fill,
            p.text_on_accent,
            p.text_on_accent.alpha(0.12),
        )
    } else {
        (p.hover, p.text_dim, p.hover)
    };
    frame.rounded(whole, fill, RADIUS_ROW);
    let open = app.menu_button == Some(crate::app::MenuButton::CommitButton);
    for (rect, target) in [
        (main, CommitTarget::Commit),
        (menu, CommitTarget::CommitMenu),
    ] {
        let hovered = !busy && app.hover == Hover::Commit(target);
        let lit = match target {
            CommitTarget::Commit => hovered && ready,
            _ => hovered || open,
        };
        if lit {
            frame.rect_clipped(rect, shade, RADIUS_ROW, whole);
        }
        hits.push((rect, target));
    }
    let divider = Rect::new(menu.x, menu.y + 7.0, 1.0, menu.h - 14.0);
    frame.rect_clipped(divider, ink.alpha(ink.a * 0.3), 0.0, whole);
    icons::chevron(frame, center(menu), 6.0, Direction::Down, ink, whole);
    // 合并、变基暂停时按钮用于完成当前操作；刚提交成功时短暂显示新提交的提交号。
    let text = match (busy, operation, app.commit.amend) {
        _ if let Some((oid, _)) = done => t!("git-commit-button-committed", oid = oid.as_str()),
        _ if syncing => t!("git-commit-button-syncing").to_owned(),
        (true, ..) => t!("git-commit-button-committing").to_owned(),
        _ if let Some((ahead, behind)) = sync => {
            // 只写有差异的方向，与推送、拉取菜单项中的箭头方向相同。
            let counts = [(ahead, "↑"), (behind, "↓")]
                .iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, arrow)| format!("{n}{arrow}"))
                .collect::<Vec<_>>()
                .join(" ");
            t!("git-commit-button-sync", counts = counts)
        }
        (false, Some(Operation::Rebase), _) => t!("git-commit-button-continue-rebase").to_owned(),
        (false, Some(Operation::Merge), _) => t!("git-commit-button-finish-merge").to_owned(),
        (false, Some(Operation::CherryPick), _) => {
            t!("git-commit-button-finish-cherry-pick").to_owned()
        }
        (false, Some(Operation::Revert), _) => t!("git-commit-button-finish-revert").to_owned(),
        (false, None, true) => t!("git-commit-button-amend").to_owned(),
        (false, None, false) => t!("git-commit-button").to_owned(),
    };
    let text = text.as_str();
    let size = app.fonts.ui_small;
    let text_w = frame.measure(FontKind::Ui, size, text);
    let icon_w = if busy && !syncing { 0.0 } else { 16.0 };
    let x = (main.x + (main.w - icon_w - text_w) / 2.0).round();
    let clip = panel.intersect(main);
    if syncing {
        // 圆形图标按大写字母高度的中心对齐：按小写字母与大写字母中心的中点对齐时，圆形看起来比
        // 文字低约 1 逻辑像素。
        let cap = app.ui_metrics.cap_height * size / app.fonts.ui;
        let cy = ui_baseline(app, main, size) - cap / 2.0;
        icons::spinner(frame, (x + 5.0, cy), spin_turn(), ink, clip);
    } else if !busy {
        let cy = icon_center_y(app, main, size);
        if sync.is_some() && done.is_none() {
            icons::refresh(frame, (x + 5.0, cy), ink, clip);
        } else {
            icons::check(frame, (x + 5.0, cy), 10.0, ink, clip);
        }
    }
    ui_text_small(frame, app, x + icon_w, main, text, ink, clip);

    caret
}

/// 加载图标每秒旋转的圈数（经验值）。
const SPIN_SPEED: f32 = 1.25;

/// 返回加载图标当前旋转到的圈数，按进程启动以来的时间计算，各处的加载图标转速一致。
fn spin_turn() -> f32 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f32()
        * SPIN_SPEED
}

/// 文件名的颜色：正文色，删掉的用次要色。
fn file_color(change: Change, p: &Palette) -> Color {
    if change == Change::Deleted {
        p.text_muted
    } else {
        p.text
    }
}

/// 改动列表中一行文件名的位置与内容。
///
/// 包括文件名起点（相对行底色左边）、文件名、平铺时其后以淡色显示的目录、文件名的颜色，
/// 以及悬停时该行的底色。
struct RowText {
    /// 文件名的起点，相对行底色左边（逻辑像素）。
    offset: f32,
    name: String,
    dir: Option<String>,
    /// 名字是并起来的几级目录（`a/b/c`），由 [`draw_dir_label`] 绘制。
    folder: bool,
    color: Color,
    /// 悬停时该行的底色；没有悬停时也是 `Some` 的行同样铺色。
    shade: Option<Color>,
}

/// 返回深 `depth` 层的文件行的勾选框在第几列缩进。
///
/// 按目录排成树时比所在目录深一层，平铺时不缩进。
fn file_column(app: &App, depth: usize) -> usize {
    if app.commit.flat() { 0 } else { depth + 1 }
}

/// 返回第 `row` 行名字的排法（组标题行返回 `None`）；列位置与 [`draw_row`] 使用同一组常量。
fn row_text(app: &App, status: &Status, root: &Path, row: &CommitRow) -> Option<RowText> {
    let p = app.palette;
    // 箭头、勾选框、图标、图标那一列宽、间隔（见 `draw_row` 和 `ARROW` 那一组常量）。
    let offset = |level: usize| {
        ARROW + level as f32 * INDENT + CHECK + ICON + icons::FOLDER_WIDTH + LABEL_GAP
    };
    match row {
        CommitRow::Group(_) => None,
        CommitRow::Dir { label, depth, .. } => Some(RowText {
            offset: offset(depth + 1),
            name: label.clone(),
            dir: None,
            folder: true,
            color: p.text,
            shade: Some(p.hover),
        }),
        CommitRow::File { index, depth } => {
            let entry = status.entries.get(*index)?;
            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dir = entry
                .path
                .parent()
                .and_then(|p| p.strip_prefix(root).ok())
                .filter(|p| app.commit.flat() && !p.as_os_str().is_empty())
                .map(|p| p.to_string_lossy().into_owned());
            let change = entry.shown();
            let selected = app.commit_is_selected(&entry.path);
            Some(RowText {
                offset: offset(file_column(app, *depth)),
                name,
                dir,
                folder: false,
                color: file_color(change, p),
                shade: row_color(p, selected, app.focus == Focus::CommitList, true),
            })
        }
    }
}

/// 计算鼠标停着的改动行写全名所需的溢出区域（见 [`CommitOverflow`]）。
///
/// 文件行的「打开文件」「回滚」两个按钮跟在全名之后。名字放得下、正在拖动时返回 `None`。
/// 展开收起的动画途中，被展开、收起的行及其上方各行位置不变，仍显示全名；下方正在移动的行
/// 返回 `None`。
pub(in crate::view) fn commit_overflow(app: &App, frame: &mut Frame<'_>) -> Option<CommitOverflow> {
    if !app.commit_shown() || !app.idle_pointer() {
        return None;
    }
    let status = app.git.status.as_ref()?;
    let root = app.git.root.as_deref()?;
    let rows = app.commit_rows();
    let row = match app.hover {
        Hover::Commit(
            CommitTarget::Dir(r) | CommitTarget::DirCheck(r) | CommitTarget::Sticky(r, _),
        ) => r,
        // 鼠标移到跟在全名后面的按钮上时，仍保持全名显示。
        Hover::Commit(
            CommitTarget::File(f)
            | CommitTarget::Check(f)
            | CommitTarget::OpenFile(f)
            | CommitTarget::RevertFile(f),
        ) => rows
            .iter()
            .position(|r| matches!(r, CommitRow::File { index, .. } if *index == f))?,
        _ => return None,
    };
    if app.commit.motion.as_ref().is_some_and(|m| row > m.parent) {
        return None;
    }
    let list = app.commit_parts().list;
    // 固定在顶部的行按固定的位置，不用滚动的偏移。
    let y = match app.commit_sticky(&rows).iter().find(|s| s.row == row) {
        Some(s) => s.rect.y,
        None => list.y + row as f32 * COMMIT_ROW - app.commit.scroll,
    };
    let text = row_text(app, status, root, rows.get(row)?)?;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    let end = name_end(app, frame, &text, band);
    let window = app.layout.window.right() - 8.0;
    let (right, actions) = if text.folder {
        if end <= band.right() - 6.0 {
            return None;
        }
        ((end + 8.0).min(window), None)
    } else {
        // 文件行的按钮在名字之后：名字与按钮在原位放得下时不展开。按钮超出窗口时向左移回，
        // 名字在按钮之前截断。
        let (open, revert) = file_actions(band);
        if end <= open.x - 4.0 {
            return None;
        }
        let shift = (end + 4.0 - open.x).min(window - revert.right());
        let moved = |r: Rect| Rect::new(r.x + shift, r.y, r.w, r.h);
        let (open, revert) = (moved(open), moved(revert));
        (revert.right() + 4.0, Some((open, revert)))
    };
    let spill = (right > list.right()).then(|| {
        Rect::new(
            list.right(),
            y + 1.0,
            right - list.right(),
            COMMIT_ROW - 2.0,
        )
    });
    Some(CommitOverflow {
        row,
        y,
        spill,
        actions,
    })
}

/// 返回 `band` 一行中名字（平铺时连同其后的目录）完整绘制时末尾的 x 坐标（逻辑像素）。
fn name_end(app: &App, frame: &mut Frame<'_>, text: &RowText, band: Rect) -> f32 {
    let name_w = if text.folder {
        dir_label_width(app, frame, &text.name)
    } else {
        frame.measure(FontKind::Ui, app.fonts.ui, &text.name)
    };
    let mut end = band.x + text.offset + name_w;
    if let Some(dir) = &text.dir {
        end += 8.0 + frame.measure(FontKind::Ui, app.fonts.ui_small, dir);
    }
    end
}

/// 在编辑区上面继续绘制放不下的改动行名字：底色、文字和列表里的那一行连成一条。
pub(in crate::view) fn draw_commit_overflow(app: &App, frame: &mut Frame<'_>) {
    let Some(CommitOverflow {
        row,
        y,
        spill: Some(spill),
        actions,
    }) = app.commit.overflow
    else {
        return;
    };
    let (Some(status), Some(root)) = (app.git.status.as_ref(), app.git.root.as_deref()) else {
        return;
    };
    let rows = app.commit_rows();
    let Some(text) = rows.get(row).and_then(|r| row_text(app, status, root, r)) else {
        return;
    };
    let p = app.palette;
    let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
    // 先垫一层侧栏底色，再叠半透明的悬停色，与列表里那一行的底色相同。
    frame.rect_corners(spill, p.sidebar_bg, radii, spill);
    if let Some(color) = text.shade {
        frame.rect_corners(spill, color, radii, spill);
    }
    let list = app.commit_parts().list;
    let band = Rect::new(list.x + 8.0, y, list.w - 16.0, COMMIT_ROW);
    let clip = Rect::new(spill.x, y, spill.w, COMMIT_ROW);
    let x = band.x + text.offset;
    // 有按钮时名字在按钮之前截断。
    let text_clip = match actions {
        Some((open, _)) => Rect::new(clip.x, clip.y, (open.x - 4.0 - clip.x).max(0.0), clip.h),
        None => clip,
    };
    let end = if text.folder {
        draw_dir_label(app, frame, (x, band), &text.name, None, text_clip)
    } else {
        ui_text(frame, app, x, band, &text.name, text.color, text_clip)
    };
    if let Some(dir) = &text.dir {
        ui_text_small(frame, app, end + 8.0, band, dir, p.text_dim, text_clip);
    }
    if let (Some(actions), CommitRow::File { index, .. }) = (actions, &rows[row]) {
        draw_file_actions(app, frame, actions, *index, clip);
    }
}

/// 返回并起来的几级目录（`Sources/DEEIXPET`）画出来的宽度（逻辑像素），见 [`draw_dir_label`]。
fn dir_label_width(app: &App, frame: &mut Frame<'_>, label: &str) -> f32 {
    let size = app.fonts.ui;
    let parts = label.split('/').count() as f32;
    let names: f32 = label
        .split('/')
        .map(|part| frame.measure(FontKind::Ui, size, part))
        .sum();
    let slash = frame.measure(FontKind::Ui, size, "/") + 2.0 * CHAIN_GAP;
    names + (parts - 1.0) * slash
}

/// 画目录行的名字。
///
/// 只有一个子目录的几级并成一行，样式与文件树中合并显示的目录相同。放不下 `room`（逻辑像素）时
/// 折叠中间的各级（见 [`fold_chain`]），目录名不会被截成半个。返回名字末尾的 x 坐标（逻辑像素）。
fn draw_dir_label(
    app: &App,
    frame: &mut Frame<'_>,
    (x, band): (f32, Rect),
    label: &str,
    room: Option<f32>,
    clip: Rect,
) -> f32 {
    let p = app.palette;
    let size = app.fonts.ui;
    let names: Vec<&str> = label.split('/').collect();
    let widths: Vec<f32> = names
        .iter()
        .map(|name| frame.measure(FontKind::Ui, size, name))
        .collect();
    let slash = frame.measure(FontKind::Ui, size, "/");
    let step = slash + 2.0 * CHAIN_GAP;
    let fold = frame.measure(FontKind::Ui, size, CHAIN_FOLD);
    let parts = match room {
        Some(room) => fold_chain(&widths, step, fold, names.len() - 1, room).0,
        None => (0..names.len()).map(Some).collect(),
    };
    let separator = p.text.alpha(p.text.a * CHAIN_SEPARATOR);
    let mut x = x;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            x += CHAIN_GAP;
            x = ui_text(frame, app, x, band, "/", separator, clip) + CHAIN_GAP;
        }
        x = match part {
            Some(index) => ui_text(frame, app, x, band, names[*index], p.text, clip),
            None => ui_text(frame, app, x, band, CHAIN_FOLD, separator, clip),
        };
    }
    x
}
