//! 侧栏绘制：第二行的项目和分支、文件树工具栏和「⋯」菜单、文件树、新建条目的输入行。
//!
//! 纵向和横向滚动、固定在顶部的目录，以及放不下的名称在编辑区上方的延续绘制都在这里处理；
//! 每一行的矩形和点击命中由 `app::tree` 在绘制时记录。

use std::f32::consts::FRAC_PI_2;

use winc_editor::git::{Change, Head};
use winc_editor::{Entry, EntryKind};
use winc_i18n::t;

use super::*;
use crate::app::{DropZone, Focus, NameInput, TreeMotion};
use crate::icons::Direction;
use crate::layout::{
    SidebarView, TREE_NAME_MIN, TreeTool, folding_tools, hscrollbar_thumb, tree_row_slot,
    tree_tools,
};

/// 绘制侧栏：
///
/// - 顶栏（窗口按钮和侧栏开关右边）：视图切换图标（文件、搜索、提交），见
///   [`panel::draw_view_tabs`]；
/// - 第二行：左边是项目和分支的上下文栏（[`draw_context_bar`]，三个视图都一样），
///   右端是当前视图的工具按钮；
/// - 下面是当前视图：文件树、全局搜索面板或提交视图。
pub(in crate::view) fn draw_sidebar(app: &mut App, frame: &mut Frame<'_>) {
    frame.rect(app.layout.sidebar, app.palette.sidebar_bg);
    panel::draw_view_tabs(app, frame);

    if app.sidebar_view == SidebarView::Search {
        let right = app.panel_parts().tools[0].0.x - 4.0;
        let bar = ContextBar::new(app, frame);
        draw_context_bar(app, frame, &bar, right);
        if let Some(caret) = panel::draw_panel(app, frame) {
            app.ime_cursor = Some(caret);
        }

        return;
    }

    if app.sidebar_view == SidebarView::Commit {
        let right = app.commit_parts().tools[0].0.x - 4.0;
        let bar = ContextBar::new(app, frame);
        draw_context_bar(app, frame, &bar, right);
        if let Some(caret) = git::commit::draw_commit(app, frame) {
            app.ime_cursor = Some(caret);
        }

        return;
    }

    draw_project_header(app, frame);
    draw_tree(app, frame);
}

/// 绘制文件视图的第二行：左边是上下文栏（项目、当前分支），右端是文件树工具栏。
fn draw_project_header(app: &mut App, frame: &mut Frame<'_>) {
    let header = app.layout.sidebar_header;
    let bar = ContextBar::new(app, frame);

    // 侧栏变窄时先收工具栏、后截名字：为完整的项目名留出宽度（分支不占宽度：放不下时不再绘制，
    // 状态栏里始终显示），工具栏放不下时从右向左把按钮收进「⋯」菜单；只剩「⋯」时才截短名字，
    // 项目名截到最小宽度（`TREE_NAME_MIN`）为止，它是切换目录的入口，必须始终可点击。
    (app.tree_tools, app.tree_more) = if bar.opened {
        let only_more = header.right() - 8.0 - BUTTON_SMALL - 4.0;
        let name_right = (bar.left + bar.project_full() + 8.0)
            .min(only_more)
            .max(bar.left + bar.name_offset() + TREE_NAME_MIN);
        tree_tools(header, name_right)
    } else {
        (Vec::new(), None)
    };
    app.step_tool_fold();
    let right = app
        .tree_tools
        .first()
        .map(|(button, _)| button.x)
        .or(app.tree_more.map(|more| more.x))
        .map_or(header.right() - 8.0, |x| x - 4.0);

    draw_context_bar(app, frame, &bar, right);
    draw_tree_tools(app, frame);
}

/// 项目名和分支之间的空隙（逻辑像素）。
const CONTEXT_GAP: f32 = 2.0;
/// 文件树中目录有 Git 改动时，右端圆点的直径（逻辑像素）。
const GIT_DOT: f32 = 6.0;
/// 文件树中放不下的名称在侧栏边缘淡出的宽度（逻辑像素），
/// 与结果行上按钮左侧的渐隐同宽。
const NAME_FADE: f32 = 20.0;
/// 分支按钮里图标占的宽度（逻辑像素）：左边距加分支图标。
const BRANCH_HEAD: f32 = 20.0;
/// 分支按钮里文字右边到按钮右边的宽度（逻辑像素）：空隙加下拉箭头。
const BRANCH_TAIL: f32 = 18.0;
/// 项目按钮里徽章占的宽度（逻辑像素）：左边 6、徽章 20、右边 8。
const BADGE_ROOM: f32 = 34.0;

/// 侧栏第二行左边的上下文栏：现在在哪个项目、哪个分支上。
///
/// 项目按钮三个视图（文件、搜索、提交）都一样，切换视图时不动：首字母徽章、项目名和下拉箭头，
/// 点了弹项目菜单（最近的目录、打开目录、关闭目录）。提交视图里、放得下时后面紧跟着当前分支
/// 按钮，分支图标、分支名（游离时是提交号）和下拉箭头，点了弹分支菜单，和状态栏的分支名是同一个
/// 菜单；文件、搜索视图里和放不下时不绘制（状态栏里一直有）。
pub(super) struct ContextBar {
    /// 项目名；没有打开目录时是「未打开目录」。
    project: String,
    /// 打开了目录（可以绘制徽章，名称用正文色）。
    opened: bool,
    /// 当前分支；不在 Git 仓库里时为 `None`。
    branch: Option<String>,
    /// 项目按钮的左边。
    left: f32,
    /// 项目名、分支名完整显示时的文字宽度。
    project_w: f32,
    branch_w: f32,
}

impl ContextBar {
    /// 读出当前的项目名和分支，并量出两者完整显示时的文字宽度。
    pub(super) fn new(app: &App, frame: &mut Frame<'_>) -> Self {
        let opened = app.editor.workspace.is_some();
        let project = app.editor.workspace.as_ref().map_or_else(
            || t!("workspace-sidebar-no-folder").to_owned(),
            |w| w.name.clone(),
        );
        // 分支只在提交视图中绘制：提交前须能直接确认提交的目标分支。文件、搜索视图中分支与当前操作
        // 无关，且状态栏右端始终显示当前分支（同一个菜单），因此不重复绘制。
        let branch = (opened && app.sidebar_view == SidebarView::Commit)
            .then(|| app.git.status.as_ref()?.head.as_ref())
            .flatten()
            .map(|head| match head {
                Head::Branch(name) => name.clone(),
                Head::Detached(oid) => format!("({oid})"),
            });
        let project_w = frame.measure(FontKind::Ui, app.fonts.ui, &project);
        let branch_w = branch
            .as_deref()
            .map_or(0.0, |b| frame.measure(FontKind::Ui, app.fonts.ui_small, b));
        ContextBar {
            project,
            opened,
            branch,
            left: app.layout.sidebar_header.x + 8.0,
            project_w,
            branch_w,
        }
    }

    /// 项目名文字相对项目按钮左边的偏移：打开了目录时让出徽章（徽章一直在，名字放不下时
    /// 截名字，不省徽章），否则留 6。
    fn name_offset(&self) -> f32 {
        if self.opened { BADGE_ROOM } else { 6.0 }
    }

    /// 项目按钮完整显示时的宽度。
    fn project_full(&self) -> f32 {
        self.name_offset() + self.project_w + 18.0
    }

    /// 分支按钮完整显示时的宽度；没有分支时为 0。
    fn branch_full(&self) -> f32 {
        match self.branch {
            Some(_) => BRANCH_HEAD + self.branch_w + BRANCH_TAIL,
            None => 0.0,
        }
    }

    /// 上下文栏有 `room` 宽时项目按钮、分支按钮各占多宽（分支为 0 表示不显示）。
    ///
    /// 项目名优先：两者都完整放得下时才绘制分支，否则不绘制分支（状态栏右端始终写着当前分支，
    /// 点击打开同一个菜单），项目名也不必为它截短。
    fn split(&self, room: f32) -> (f32, f32) {
        let (project, branch) = (self.project_full(), self.branch_full());
        if self.branch.is_some() && project + CONTEXT_GAP + branch <= room {
            (project, branch)
        } else {
            (project.min(room), 0.0)
        }
    }
}

/// 在第二行左边绘制上下文栏（[`ContextBar`]），右边不超过 `right`；记下项目按钮、分支按钮的
/// 位置（`project_hit`、`branch_hit`）。
pub(super) fn draw_context_bar(app: &mut App, frame: &mut Frame<'_>, bar: &ContextBar, right: f32) {
    let p = app.palette;
    let header = app.layout.sidebar_header;
    let clip = header.intersect(app.layout.sidebar);
    let (project_w, branch_w) = bar.split((right - 4.0 - bar.left).max(0.0));
    let name_x = bar.left + bar.name_offset();

    // 项目：徽章一直在；名称太长时从末尾截短，其后是下拉箭头。
    let badge_rect = Rect::new(
        bar.left + 6.0,
        header.y + (header.h - 20.0) / 2.0,
        20.0,
        20.0,
    );
    let name_max = (bar.left + project_w - 18.0 - name_x).max(0.0);
    let name = truncate_end(frame, &bar.project, app.fonts.ui, name_max);
    let name_end = name_x + frame.measure(FontKind::Ui, app.fonts.ui, &name);
    let hit = Rect::new(
        bar.left,
        header.y + (header.h - BUTTON_SMALL) / 2.0,
        name_end + 18.0 - bar.left,
        BUTTON_SMALL,
    );
    let open = app.menu_button == Some(MenuButton::Project);
    draw_button(frame, p, hit, clip, app.hover == Hover::ProjectName, open);
    if bar.opened {
        draw_badge(app, frame, badge_rect, &bar.project, clip);
    }
    let color = if bar.opened { p.text } else { p.text_muted };
    let x = ui_text(frame, app, name_x, header, &name, color, clip);
    let arrow = (x + 9.0, icon_center_y(app, header, app.fonts.ui));
    icons::chevron(frame, arrow, 6.5, Direction::Down, p.text_muted, clip);
    app.project_hit = hit.intersect(clip);

    // 分支：比项目名小一号、淡一档，看得出是项目下面的一层。
    let Some(branch) = bar.branch.as_deref().filter(|_| branch_w > 0.0) else {
        return;
    };
    // 只在完整放得下时才绘制（`ContextBar::split`），名字不截短。
    let pill = Rect::new(hit.right() + CONTEXT_GAP, hit.y, branch_w, hit.h);
    let hovered = app.hover == Hover::BranchName;
    let open = app.menu_button == Some(MenuButton::Branch);
    draw_button(frame, p, pill, clip, hovered, open);
    let cy = icon_center_y(app, header, app.fonts.ui_small);
    let color = if hovered || open {
        p.text
    } else {
        p.text_muted
    };
    icons::branch_small(frame, (pill.x + 11.0, cy), color, clip);
    let end = label(
        frame,
        app,
        app.fonts.ui_small,
        (pill.x + BRANCH_HEAD, header),
        branch,
        color,
        clip,
    );
    icons::chevron(
        frame,
        (end + 9.0, cy),
        5.5,
        Direction::Down,
        p.text_dim,
        clip,
    );
    app.branch_hit = pill.intersect(clip);
}

/// 绘制项目名前面的徽章：圆角方块内是项目名的前两个字母或数字（大写），没有时用「W」。
///
/// 打开的是附属工作树时，徽章用主工作树的名字和颜色（看得出是哪个仓库；名字写的是工作树自己
/// 的目录，分得出是哪一个），右下角加一个工作树记号。
fn draw_badge(app: &App, frame: &mut Frame<'_>, rect: Rect, project: &str, header: Rect) {
    let main = app.git.main_worktree();
    let (project, color) = match main {
        Some(main) => (
            crate::app::git::worktree::worktree_name(main),
            app.state.badge_color(&main.path),
        ),
        None => (
            project.to_owned(),
            app.editor
                .root()
                .map_or(0, |root| app.state.badge_color(root)),
        ),
    };
    let badge = app.palette.project_badge(&project, color);
    frame.rect_clipped(rect, badge.background, 6.0, header);

    let w = frame.measure(FontKind::Ui, app.fonts.initials, &badge.initials);
    caps_label(
        frame,
        app,
        app.fonts.initials,
        (rect.x + (rect.w - w) / 2.0, rect),
        &badge.initials,
        badge.text,
        header,
    );
    if main.is_some() {
        let colors = (app.palette.sidebar_bg, badge.background, badge.text);
        icons::worktree_mark(
            frame,
            (rect.right() - 1.0, rect.bottom() - 1.0),
            colors,
            header,
        );
    }
}

/// 绘制项目名右侧的文件树工具栏：新建文件、新建目录、刷新、全部折叠、定位当前文件；
/// 放不下的按钮收进「⋯」按钮。
///
/// 按收起动画的进度排布（`folding_tools`）：收起的按钮滑进「⋯」并淡出，展开时反过来；
/// 动画途中压到项目名上的部分不绘制。
fn draw_tree_tools(app: &App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let header = app.layout.sidebar_header;
    let left = app.project_hit.right().max(app.branch_hit.right()) + BUTTON_GAP;
    let clip = Rect::new(left, header.y, (header.right() - left).max(0.0), header.h);
    let (shown, more_shown) = app.tool_fold.eased();
    let (tools, more) = folding_tools(header, &shown, more_shown);

    for (button, tool, shown) in tools {
        let kept = app.tree_tools.iter().any(|&(_, t)| t == tool);
        let hovered = kept && app.hover == Hover::TreeTool(tool);
        let color = if hovered { p.text } else { p.text_muted };
        // 透明度比位置变化得快：滑到「⋯」上时已经接近透明，几个图标叠在一起也不会混成一片。
        frame.with_opacity(shown * shown, |frame| {
            draw_button(frame, p, button, clip, hovered, false);
            draw_tree_tool_icon(frame, tool, center(button), color, button.intersect(clip));
        });
    }

    if let Some(more) = more {
        // 菜单打开时按钮显示为按下状态。
        let open = app.menu_button == Some(MenuButton::TreeMore);
        let hovered = app.tree_more.is_some() && app.hover == Hover::TreeMore;
        let color = if hovered || open {
            p.text
        } else {
            p.text_muted
        };
        frame.with_opacity(more_shown * more_shown, |frame| {
            draw_button(frame, p, more, clip, hovered, open);
            icons::more(frame, center(more), color, more.intersect(clip));
        });
    }
}

/// 绘制工具 `tool` 的图标，中心在 `center`，裁剪到 `clip`。
fn draw_tree_tool_icon(
    frame: &mut Frame<'_>,
    tool: TreeTool,
    center: (f32, f32),
    color: Color,
    clip: Rect,
) {
    match tool {
        TreeTool::NewFile => icons::add_file(frame, center, color, clip),
        TreeTool::NewFolder => icons::add_folder(frame, center, color, clip),
        TreeTool::Refresh => icons::refresh(frame, center, color, clip),
        TreeTool::CollapseAll => icons::collapse_all(frame, center, color, clip),
        TreeTool::Locate => icons::locate(frame, center, color, clip),
    }
}

/// 绘制文件树：选中行和悬停行是内缩的圆角块。
fn draw_tree(app: &mut App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let tree = app.layout.tree;

    let Some(ws) = app.editor.workspace.as_ref() else {
        // 顶部已经显示「未打开目录」，这里只提示打开方式，并与该文本左对齐。
        let row = Rect::new(tree.x, tree.y + 4.0, tree.w, TREE_ROW);
        let x = app.layout.sidebar_header.x + 14.0;
        let hint = match keymap::shortcut(Action::OpenFile) {
            Some(key) => t!("workspace-tree-open-hint-key", key = key),
            None => t!("workspace-tree-open-hint").to_owned(),
        };
        ui_text_small(frame, app, x, row, &hint, p.text_dim, tree);
        return;
    };

    let rows = ws.rows();

    // 目录为空时居中显示一句提示，说明新建文件的方式，与搜索、提交视图的空列表相同；
    // 双击空白处即在根目录新建文件（见 `click_tree`）。正在输入新建的名称时不显示提示。
    let creating = app.name_input.as_ref().is_some_and(|e| e.rename.is_none());
    if rows.is_empty() && !creating {
        let span = (tree, sidebar_span(app));
        let (title, hint) = (t!("workspace-tree-empty"), t!("workspace-tree-empty-hint"));
        draw_empty_state(app, frame, span, title, Some(hint));
    }

    // 新建文件、目录时，在目标目录子树的第一行插入一行输入名称的输入行；重命名时输入框
    // 绘制在原条目所在的行上，不另插一行。输入框先从 `app` 中取出，绘制后放回，
    // 这样循环中读取 `app` 时不需要复制整棵文件树。
    let mut name_input = app.name_input.take();
    let editing = name_input.as_ref().map(|e| e.index);
    let inserted = name_input
        .as_ref()
        .filter(|e| e.rename.is_none())
        .map(|e| e.index);
    let mut ime = None;

    // 目录展开或收起的动画中，按展开后的行序排布：收起时把正在收起的子行插回原位。子行区域
    // 停在原位，只露出 `reveal` 所示比例的部分；其下的行向上移动 `hidden`（逻辑像素）的高度。
    let motion = app.tree_motion.as_ref().filter(|m| m.parent < rows.len());
    let block = motion.map(TreeMotion::block);
    let removed: &[Entry] = motion.map_or(&[], |m| m.removed.as_slice());
    let reveal = motion.map_or(1.0, TreeMotion::reveal);
    let hidden = motion.map_or(0.0, TreeMotion::hidden);
    let total = rows.len() + removed.len() + usize::from(inserted.is_some());

    // 内容高度与点击、拖动滚动条使用的相同（见 `tree_max_scroll`：末尾多留一行），动画中减去
    // 尚未露出的高度。行数变少之后（收起目录、外部删除了文件），滚动位置在这里夹回有效范围；
    // 收起动画中，滚动位置随露出的高度平滑回收。
    let content = (total + 1) as f32 * TREE_ROW - hidden;
    let scroll = app.tree_scroll.clamp(0.0, (content - tree.h).max(0.0));
    // 固定在顶部的目录按该滚动位置计算，因此先写回 `app.tree_scroll`。
    app.tree_scroll = scroll;

    // 返回按展开后的行序数的第 `v` 行对应的条目，以及该条目在文件树数据中的行号；正在收起的子行
    // 已不在文件树数据中，行号为 `None`。
    let entry_at = |v: usize| -> (&Entry, Option<usize>) {
        match block {
            Some((start, end)) if !removed.is_empty() && v >= start => {
                if v < end {
                    (&removed[v - start], None)
                } else {
                    (&rows[v - removed.len()], Some(v - removed.len()))
                }
            }
            _ => (&rows[v], Some(v)),
        }
    };

    // 拖动时（也包括从系统文件管理器拖着文件经过），作为放置目标的目录连同其已展开的子行铺一层
    // 底色；目标是根目录时覆盖整棵树。拖动期间的展开只会是停在目录上触发的自动展开，不存在正在
    // 收起的子行，因此 `removed` 总是空的。
    let target = match app.file_hover() {
        Some(DropZone::Tree(dir)) => Some(dir),
        _ => app.tree_dragging().and_then(|d| d.target.as_ref()),
    };
    if let Some(dir) = target.filter(|_| removed.is_empty()) {
        let area = if *dir == ws.root {
            Some(tree.inset(8.0, 2.0))
        } else {
            ws.row_of(dir).map(|start| {
                let count = rows[start].end - start - 1;
                let (top, _) = tree_row_slot(tree, scroll, start, block, hidden);
                let (last, clip) = tree_row_slot(tree, scroll, start + count, block, hidden);
                let bottom = last.bottom().min(clip.bottom());
                Rect::new(
                    tree.x + 8.0,
                    top.y + 1.0,
                    tree.w - 16.0,
                    bottom - top.y - 2.0,
                )
            })
        };
        if let Some(area) = area {
            frame.rect_clipped(area, p.drop_target, RADIUS_ROW, tree);
        }
    }

    let first = (scroll / TREE_ROW).floor().max(0.0) as usize;
    let visible = (tree.h / TREE_ROW).ceil() as usize + 1 + (hidden / TREE_ROW).ceil() as usize;
    // 横向滚动使缩进、图标和名称一起左移；底色的圆角矩形不动。
    let shift = -app.tree_scroll_x;
    let mut widest: f32 = 0.0;

    for v in first..(first + visible).min(total) {
        let (row, row_clip) = tree_row_slot(tree, scroll, v, block, hidden);

        // 输入行（重命名时是原条目所在的行）改为绘制输入框，不绘制条目。
        if editing == Some(v) {
            if let Some(entry) = name_input.as_mut() {
                draw_name_input(app, frame, entry, row, row_clip, shift, &mut ime);
            }

            continue;
        }

        // 输入行占用一个显示槽位，其后的条目在数据中的行号比显示槽位号小 1，位置不变。
        let (entry, index) = entry_at(match inserted {
            Some(at) if v > at => v - 1,
            _ => v,
        });

        if row.bottom() <= row_clip.y || row.y >= row_clip.bottom() {
            continue;
        }

        // 正在展开或收起的目录，箭头随动画进度转动。
        let open = match motion {
            Some(m) if index == Some(m.parent) => reveal,
            _ if entry.expanded => 1.0,
            _ => 0.0,
        };
        let width = draw_tree_entry(app, frame, entry, index, row, row_clip, open);
        widest = widest.max(width);
    }

    draw_sticky(app, frame, rows);

    draw_scrollbar(app, frame, ScrollArea::Tree, 0, tree, scroll, content);

    // 最宽的一行决定横向能滚多远，只统计当前屏幕上的可见行。末尾留出 20 逻辑像素，使滚到头时
    // 名称不贴着侧栏边缘。收起目录、刷新之后该宽度会变小，横向滚动位置随之回收。
    app.tree_content_w = widest + 20.0;
    app.clamp_tree_scroll_x();
    app.name_input = name_input;
    app.ime_cursor = ime;

    let thumb = hscrollbar_thumb(tree, app.tree_scroll_x, app.tree_content_w);
    let look = app.scrollbar_look(
        ScrollArea::TreeHorizontal,
        0,
        app.tree_scroll_x,
        thumb.map(|_| tree),
    );
    if let Some(thumb) = thumb {
        draw_thumb(frame, p, thumb, look);
    }
}

/// 绘制文件树的一行：选中或悬停的底色、缩进参考线、箭头、图标和名称。
///
/// `open` 是目录箭头已转过的比例（0 为收起，1 为展开）。返回从行左边缘到名称末尾的宽度（逻辑像素，
/// 不含横向滚动偏移），横向滚动范围据此计算。
fn draw_tree_entry(
    app: &App,
    frame: &mut Frame<'_>,
    entry: &Entry,
    index: Option<usize>,
    row: Rect,
    clip: Rect,
    open: f32,
) -> f32 {
    let p = app.palette;
    let tree = app.layout.tree;
    let shift = -app.tree_scroll_x;
    let ws = app.editor.workspace.as_ref();
    let selected = index.is_some_and(|i| ws.is_some_and(|w| w.is_selected(i)));
    let hovered = index.is_some_and(|i| {
        app.hover == Hover::TreeRow(i) || app.tree_overflow.is_some_and(|(_, o)| o == i)
    });

    // 名称放不下时，底色一直铺到侧栏边缘、右侧不留圆角，看起来像被侧栏边缘截断；
    // 鼠标移上去时在编辑区上方继续绘制完整名称（见 `draw_tree_overflow`）。
    let label_x = tree_label_x(row, entry.depth, shift);
    let label_end = label_x + frame.measure(FontKind::Ui, app.fonts.ui, &entry.name);
    let overflows = label_overflows(tree, label_end);
    let pill = tree_row_pill(row, overflows);

    let background = row_color(p, selected, app.focus == Focus::Tree, hovered);
    if let Some(color) = background {
        frame.rect_corners(pill.0, color, pill.1, clip);
    }

    let indent = row.x + 14.0 + shift;
    let cy = icon_center_y(app, row, app.fonts.ui);

    // 缩进参考线：每级一条，位于父目录箭头的正下方，相邻行的参考线连成竖线。
    for level in 0..entry.depth {
        let x = (indent + level as f32 * TREE_INDENT + 4.0).round();
        frame.rect_clipped(Rect::new(x, row.y, 1.0, row.h), p.indent_guide, 0.0, clip);
    }

    let mut x = indent + entry.depth as f32 * TREE_INDENT;

    if entry.kind == EntryKind::Dir {
        let angle = open * FRAC_PI_2;
        icons::chevron_turned(frame, (x + 4.0, cy), 6.0, angle, p.text_dim, clip);
    }

    x += 14.0;

    if entry.kind == EntryKind::Dir {
        let top = (cy - 6.0).round();
        icons::folder(frame, (x, top), entry.expanded, p.folder, clip);
    } else {
        draw_file_icon(frame, p, (x + 1.5, cy), &entry.name, clip);
    }

    // 右端的 Git 状态与提交视图相同：文件显示带颜色的字母（M、U、A……），目录中有改动时
    // 显示圆点。名称会被状态截断的行在悬停时不绘制状态，以完整显示名称；
    // 放不下的部分继续绘制在编辑区上方。
    let change = app.git.change_of(&entry.path);
    let letter = change
        .filter(|_| entry.kind != EntryKind::Dir)
        .map(|c| c.letter().to_string());
    let mark_w = match (&letter, change) {
        (Some(letter), _) => frame.measure(FontKind::Ui, app.fonts.ui_small, letter),
        (None, Some(_)) => GIT_DOT,
        (None, None) => 0.0,
    };
    let mark_x = tree.right() - 16.0 - mark_w;
    let change = change.filter(|_| !(hovered && label_end > mark_x - 6.0));
    // 名字画到哪里为止：有状态时停在状态左边，没有时到侧栏边缘。
    let (text_right, cut) = match change {
        Some(_) => (mark_x - 6.0, label_end > mark_x - 6.0),
        None => (tree.right(), overflows),
    };

    let color = tree_entry_color(app, entry, selected);
    let text_clip = Rect::new(clip.x, clip.y, (text_right - clip.x).max(0.0), clip.h);
    ui_text(
        frame,
        app,
        label_x,
        row,
        &entry.name,
        color,
        text_clip.intersect(clip),
    );

    // 截断的名字逐渐淡出，避免在半个字处直接截断。悬停时不淡：完整名称继续绘制在编辑区上方
    // （[`draw_tree_overflow`]），淡出会使两段接不上。
    if cut && !hovered {
        let solid = background.map_or(p.sidebar_bg, |c| panel::over(p.sidebar_bg, c));
        let edge = Rect::new(text_right - NAME_FADE, row.y, NAME_FADE, row.h);
        frame.within(clip, |frame| fade(frame, edge, solid, false));
    }

    if let Some(change) = change {
        let color = git_color(p, change);
        match &letter {
            Some(letter) => {
                ui_text_small(frame, app, mark_x, row, letter, color, clip);
            }
            None => {
                let dot = Rect::new(mark_x, cy - GIT_DOT / 2.0, GIT_DOT, GIT_DOT);
                frame.rect_clipped(dot, color, GIT_DOT / 2.0, clip);
            }
        }
    }

    label_end - shift - row.x
}

/// Git 状态在文件树、变更视图里用的颜色。
pub(in crate::view) fn git_color(p: &Palette, change: Change) -> Color {
    match change {
        Change::Modified | Change::Renamed => p.git_modified,
        Change::Added | Change::Untracked => p.git_added,
        Change::Deleted => p.git_deleted,
        Change::Conflicted => p.git_conflict,
    }
}

/// 绘制固定在文件树顶部的各级目录：覆盖在滚到上方的行之上，下沿一道淡阴影把它们与滚动的行分开。
///
/// 被推上去的那一层压在上一层之下，因此按从内层到外层的顺序绘制。
fn draw_sticky(app: &App, frame: &mut Frame<'_>, rows: &[Entry]) {
    let sticky = app.tree_sticky();
    let Some(bottom) = sticky.iter().map(|s| s.rect.bottom()).reduce(f32::max) else {
        return;
    };
    let p = app.palette;
    let tree = app.layout.tree;

    frame.within(tree, |frame| {
        let header = Rect::new(tree.x, tree.y, tree.w, bottom - tree.y);
        let covered = frame.covered(header, 0.0);
        frame.shadow(header, 0.0, 6.0, 0.0, p.shadow, Some(covered));

        for s in sticky.iter().rev() {
            frame.rect_clipped(s.rect, p.sidebar_bg, 0.0, tree);
            draw_tree_entry(app, frame, &rows[s.index], Some(s.index), s.rect, tree, 1.0);
        }
    });
}

/// 绘制文件树中正在输入名称的那一行：图标、输入框，出错时右侧显示原因；光标矩形写入 `ime`。
///
/// 位置与普通行一致（缩进、图标都在相同的位置），使该行看起来就是目录中即将出现的一项。
fn draw_name_input(
    app: &App,
    frame: &mut Frame<'_>,
    entry: &mut NameInput,
    row: Rect,
    row_clip: Rect,
    shift: f32,
    ime: &mut Option<Rect>,
) {
    let p = app.palette;
    let depth = entry.depth;
    let kind = entry.kind;
    let error = entry.error.clone();
    let cy = icon_center_y(app, row, app.fonts.ui);
    // 正在输入的这一行铺一层圆角底色（与其他行的底色一样大），可以看出在编辑哪一行；出错时
    // 底色外加一圈红色细线，出错原因写在底色内右侧，与红线连在一起。不另绘制一个输入框：树里
    // 一行很窄，再套一个框显得拥挤。
    let text_x = tree_label_x(row, depth, shift);
    let focused = app.focus == Focus::Prompt;
    let (pill, radii) = tree_row_pill(row, false);
    if error.is_some() {
        let ring = radii.map(|r| r + 1.0);
        frame.rect_corners(pill.inset(-1.0, -1.0), p.error, ring, row_clip);
    }
    // 用悬停的浅色而不是选中色：重命名时文件名主干是选中的，选中色底上看不出文字的选区。
    // 先和侧栏底色混合成不透明的颜色，盖住底下的红线。
    let fill = super::panel::over(p.sidebar_bg, p.hover);
    frame.rect_corners(pill, fill, radii, row_clip);

    // 缩进和图标按普通行的位置放置，与上下相邻的条目对齐。
    let indent = row.x + 14.0 + shift;
    let x = indent + depth as f32 * TREE_INDENT + 14.0;

    // 缩进参考线：与普通行相同，每级一条。
    for level in 0..depth {
        let guide = (indent + level as f32 * TREE_INDENT + 4.0).round();
        frame.rect_clipped(
            Rect::new(guide, row.y, 1.0, row.h),
            p.indent_guide,
            0.0,
            row_clip,
        );
    }

    // 重命名目录时仍然绘制箭头，使该行看起来就是原来的那一行变成了输入框。
    let open = match entry.rename {
        Some((_, expanded)) => {
            if kind == EntryKind::Dir {
                let angle = if expanded { FRAC_PI_2 } else { 0.0 };
                let arrow = (x - 14.0 + 4.0, cy);
                icons::chevron_turned(frame, arrow, 6.0, angle, p.text_dim, row_clip);
            }
            expanded
        }
        None => true,
    };

    if kind == EntryKind::Dir {
        icons::folder(frame, (x, (cy - 6.0).round()), open, p.folder, row_clip);
    } else {
        draw_file_icon(frame, p, (x + 1.5, cy), entry.field.text(), row_clip);
    }

    // 出错时底色里右侧留出显示原因的位置，文字只使用剩余宽度。
    let error_room = error.as_ref().map_or(0.0, |e| {
        frame.measure(FontKind::Ui, app.fonts.ui_small, e) + 16.0
    });
    let width = (pill.right() - 4.0 - error_room - text_x).max(0.0);
    let area = Rect::new(text_x, row.y, width, row.h);
    let caret = draw_bar_field(
        app,
        frame,
        area,
        &mut entry.field,
        if entry.rename.is_some() {
            t!("workspace-tree-rename-placeholder")
        } else {
            t!("workspace-tree-create-placeholder")
        },
        focused,
    );

    if let Some(error) = &error {
        let x = pill.right() - 8.0 - frame.measure(FontKind::Ui, app.fonts.ui_small, error);
        ui_text_small(frame, app, x, row, error, p.error, row_clip);
    }

    *ime = caret;
}

/// 返回文件树一行内名称起点的 x 坐标：位于缩进、箭头和图标之后，再加上横向滚动偏移
/// `shift`（为负值）。
///
/// 目录和文件的图标同宽。
fn tree_label_x(row: Rect, depth: usize, shift: f32) -> f32 {
    row.x + 48.0 + depth as f32 * TREE_INDENT + shift
}

/// 是否名称末尾 `label_end` 超出了该行圆角底色的右边界。
fn label_overflows(tree: Rect, label_end: f32) -> bool {
    label_end > tree.right() - 12.0
}

/// 返回一行的圆角底色矩形和四个角的半径。
///
/// 名称放不下（`overflows`）时底色铺到侧栏边缘，右侧为直角。
fn tree_row_pill(row: Rect, overflows: bool) -> (Rect, [f32; 4]) {
    let (y, h) = (row.y + 1.0, row.h - 2.0);

    if overflows {
        let r = RADIUS_ROW;
        (Rect::new(row.x + 8.0, y, row.w - 8.0, h), [r, 0.0, 0.0, r])
    } else {
        (Rect::new(row.x + 8.0, y, row.w - 16.0, h), [RADIUS_ROW; 4])
    }
}

/// 返回文件树中 `entry` 名称的颜色：编辑区中当前打开的文件用强调色标出（已选中时除外）；
/// 其余有 Git 状态的用状态色，被 Git 忽略的用淡色，都没有的用正文色。
///
/// 按住另一个文件、尚未松开时，当前打开的文件不用强调色：选中在按下时移走，文件在松开时才
/// 打开，其间若改用强调色，该文件会在按下与松开之间多一次颜色变化。
///
/// 文件树中的行与放不下时在编辑区上方继续绘制的部分共用该颜色，使两段颜色连续。
fn tree_entry_color(app: &App, entry: &Entry, selected: bool) -> Color {
    let p = &app.palette;
    let active = app.editor.active().and_then(|d| d.path.as_deref());
    let is_active = !app.tree_open_pending() && active == Some(entry.path.as_path());
    let color = match app.git.change_of(&entry.path) {
        Some(change) if !is_active || selected => git_color(p, change),
        _ if is_active && !selected => p.accent,
        // 被 Git 忽略的条目（`target/`、`node_modules/` 及其中的项）用淡色绘制，
        // 表示其不受版本控制。
        _ if app.git.is_ignored(&entry.path) => p.text_dim,
        _ => p.text,
    };
    // 剪切了、还没粘贴的项绘制得更淡。
    if app.shared.cut_files.borrow().contains(&entry.path) {
        color.alpha(color.a * 0.5)
    } else {
        color
    }
}

/// 返回鼠标所在的文件树行在名称放不下时，于编辑区上方继续绘制的那一段：矩形和该行的行号。
///
/// 不需要接续绘制时返回 `None`。
pub(in crate::view) fn tree_overflow(app: &App, frame: &mut Frame<'_>) -> Option<(Rect, usize)> {
    let index = app.overflow_candidate()?;
    let entry = app.editor.workspace.as_ref()?.rows().get(index)?;
    let tree = app.layout.tree;
    // 固定在顶部的目录在固定的位置继续绘制。
    let row = match app.tree_sticky().into_iter().find(|s| s.index == index) {
        Some(sticky) => sticky.rect,
        None => tree_row_slot(tree, app.tree_scroll, app.tree_slot_of(index), None, 0.0).0,
    };
    let shift = -app.tree_scroll_x;
    let end = tree_label_x(row, entry.depth, shift)
        + frame.measure(FontKind::Ui, app.fonts.ui, &entry.name);

    if !label_overflows(tree, end) {
        return None;
    }

    let right = (end + 8.0).min(app.layout.window.right() - 8.0);
    let rect = Rect::new(tree.right(), row.y + 1.0, right - tree.right(), row.h - 2.0);
    (rect.w > 0.0 && app.mouse().0 < rect.right()).then_some((rect, index))
}

/// 在编辑区上方继续绘制放不下的文件名：底色和文本都与文件树中的那一行连成一体。
pub(in crate::view) fn draw_tree_overflow(app: &App, frame: &mut Frame<'_>) {
    let (Some((rect, index)), Some(ws)) = (app.tree_overflow, &app.editor.workspace) else {
        return;
    };
    let Some(entry) = ws.rows().get(index) else {
        return;
    };

    let p = app.palette;
    let tree = app.layout.tree;
    let clip = Rect::new(rect.x, tree.y, rect.w, tree.h);
    let radii = [0.0, RADIUS_ROW, RADIUS_ROW, 0.0];
    let selected = ws.is_selected(index);
    let focused = app.focus == Focus::Tree;

    // 底层先垫一层侧栏底色，再叠半透明的选中色，使颜色与文件树中的那一段一致。
    frame.rect_corners(rect, p.sidebar_bg, radii, clip);

    if let Some(color) = row_color(p, selected, focused, true) {
        frame.rect_corners(rect, color, radii, clip);
    }

    let row = Rect::new(tree.x, rect.y - 1.0, tree.w, rect.h + 2.0);
    let color = tree_entry_color(app, entry, selected);
    let x = tree_label_x(row, entry.depth, -app.tree_scroll_x);
    ui_text(frame, app, x, row, &entry.name, color, clip);
}

/// 绘制侧栏开关按钮；侧栏展开时图标显示为展开状态。
pub(in crate::view) fn draw_sidebar_button(app: &App, frame: &mut Frame<'_>) {
    let p = app.palette;
    let button = app.layout.sidebar_button;
    let open = app.sidebar_open();

    let hovered = app.hover == Hover::SidebarButton;
    draw_button(frame, p, button, app.layout.window, hovered, false);
    icons::sidebar(frame, center(button), open, p.text_muted, button);
}
