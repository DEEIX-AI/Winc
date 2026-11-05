//! 分支选择器：点分支名时在快速打开面板里列出分支，可以搜索，按 `/` 分层。
//!
//! 第一行是「Git 操作」菜单，之后依次是最近使用、本地分支和远程分支。输入查询时只列出匹配的分支，
//! 目录全部展开。回车签出选中的分支，右键或行尾的「⋯」打开它的操作菜单。

use std::collections::{BTreeMap, HashSet};

use winc_editor::git::Branch;

use super::{BranchOp, local_name};
use crate::app::input::context_menu::MenuTarget;
use crate::app::input::palette::{matched, needle, rate};
use crate::app::{App, Focus, PICKER_ROW};
use winc_render::Rect;

/// 分支选择器最多同时显示的行数（行）：分支常常成组展开，比文件列表多留一些行。
pub const BRANCH_PICKER_ROWS: usize = 16;
/// 最近使用的分支最多列出几个（个）：加上标题和「Git 操作」不超过选择器一半的行数，本地分支仍在
/// 首屏。
const RECENT_BRANCHES: usize = 5;

/// [`rate`] 给字符分散命中的最低得分：低于它的是连续命中。
const SCATTERED: u32 = 1000;

/// 分支选择器的分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSection {
    /// 最近使用。
    Recent,
    /// 本地分支。
    Local,
    /// 远程分支。
    Remote,
}

/// 分支选择器的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchRow {
    /// 打开「Git 操作」菜单。
    Actions,
    /// 分组标题，不能选中。
    Heading(BranchSection),
    /// 分支名按 `/` 分出的一层目录。
    Folder {
        /// 是否在远程分支一组。
        remote: bool,
        /// 从分支名开头到这一层的路径。
        path: String,
        /// 显示的名字：只有一个子目录的目录与子目录合成一行。
        label: String,
        /// 缩进的层数。
        depth: usize,
        /// 目录下的分支数。
        count: usize,
        /// 是否展开。
        open: bool,
    },
    /// 一个分支。
    Branch {
        /// [`GitState::branches`](crate::app::git::GitState::branches) 的下标。
        index: usize,
        /// 缩进的层数。
        depth: usize,
        /// 显示的名字：去掉上级目录。
        label: String,
    },
}

impl BranchRow {
    /// 这一行能否选中。
    fn selectable(&self) -> bool {
        !matches!(self, BranchRow::Heading(_))
    }

    /// 两行是否是同一项：展开状态变化、列表刷新以后据此留住选中项。
    ///
    /// 分支的下标按各自所在的列表解释：`self` 是 `old` 的，`other` 是 `new` 的。
    fn same(&self, other: &BranchRow, old: &[Branch], new: &[Branch]) -> bool {
        match (self, other) {
            (BranchRow::Actions, BranchRow::Actions) => true,
            (
                BranchRow::Folder { remote, path, .. },
                BranchRow::Folder {
                    remote: r, path: p, ..
                },
            ) => remote == r && path == p,
            (
                BranchRow::Branch { index, depth, .. },
                BranchRow::Branch {
                    index: i, depth: d, ..
                },
            ) => depth == d && old.get(*index).map(|b| &b.name) == new.get(*i).map(|b| &b.name),
            _ => false,
        }
    }
}

/// 返回行 `row` 右端「⋯」按钮的矩形（窗口坐标，逻辑像素）。
pub fn more_button(row: Rect) -> Rect {
    Rect::new(
        row.right() - 34.0,
        row.y + 2.0,
        PICKER_ROW - 4.0,
        PICKER_ROW - 4.0,
    )
}

/// 排出分支选择器的各行。
///
/// `recent` 是最近切换到过的分支名（最近的在前），`query` 是 [`needle`] 处理过的查询，`toggled`
/// 是改变过默认展开状态的目录。默认只收起各个远程。
pub fn branch_rows(
    branches: &[Branch],
    recent: &[String],
    query: &[char],
    toggled: &HashSet<(bool, String)>,
) -> Vec<BranchRow> {
    let searching = !query.is_empty();
    // 有连续命中的分支时不列出字符分散命中的，否则短查询会带出大量无关分支。
    let score = |branch: &Branch| rate(query, &branch.name);
    let contiguous = searching
        && branches
            .iter()
            .any(|b| score(b).is_some_and(|s| s < SCATTERED));
    let hit =
        |branch: &Branch| !searching || score(branch).is_some_and(|s| !contiguous || s < SCATTERED);
    let mut rows = Vec::new();
    if !searching {
        rows.push(BranchRow::Actions);
        let local = |name: &&String| {
            branches
                .iter()
                .position(|b| !b.remote && !b.current && b.name == **name)
        };
        let latest: Vec<usize> = recent
            .iter()
            .filter_map(|name| local(&name))
            .take(RECENT_BRANCHES)
            .collect();
        if !latest.is_empty() {
            rows.push(BranchRow::Heading(BranchSection::Recent));
            rows.extend(latest.into_iter().map(|index| BranchRow::Branch {
                index,
                depth: 0,
                label: branches[index].name.clone(),
            }));
        }
    }
    for (remote, section) in [(false, BranchSection::Local), (true, BranchSection::Remote)] {
        let items: Vec<(usize, &str)> = branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.remote == remote && hit(b))
            .map(|(i, b)| (i, b.name.as_str()))
            .collect();
        if items.is_empty() {
            continue;
        }
        rows.push(BranchRow::Heading(section));
        let tree = Tree {
            remote,
            searching,
            toggled,
        };
        tree.push(items, "", 0, &mut rows);
    }
    rows
}

/// 排一组分支的树时不变的条件。
struct Tree<'a> {
    remote: bool,
    searching: bool,
    toggled: &'a HashSet<(bool, String)>,
}

impl Tree<'_> {
    /// 把 `items`（下标、去掉 `prefix` 的分支名）排成一层：先是目录，再是分支。
    fn push(
        &self,
        items: Vec<(usize, &str)>,
        prefix: &str,
        depth: usize,
        rows: &mut Vec<BranchRow>,
    ) {
        let mut folders: BTreeMap<&str, Vec<(usize, &str)>> = BTreeMap::new();
        let mut leaves = Vec::new();
        for (index, name) in items {
            match name.split_once('/') {
                Some((folder, rest)) => folders.entry(folder).or_default().push((index, rest)),
                None => leaves.push((index, name)),
            }
        }
        for (folder, mut sub) in folders {
            // 只有一个子目录、没有分支：和子目录合成一行。
            let mut label = folder.to_owned();
            while let Some(next) = only_folder(&sub) {
                label.push('/');
                label.push_str(next);
                for (_, rest) in &mut sub {
                    *rest = &rest[next.len() + 1..];
                }
            }
            let path = format!("{prefix}{label}");
            // 远程分支常常很多，各个远程默认收起。
            let closed = self.remote && depth == 0;
            let flipped = self.toggled.contains(&(self.remote, path.clone()));
            let open = self.searching || closed == flipped;
            rows.push(BranchRow::Folder {
                remote: self.remote,
                path: path.clone(),
                label,
                depth,
                count: sub.len(),
                open,
            });
            if open {
                self.push(sub, &format!("{path}/"), depth + 1, rows);
            }
        }
        rows.extend(leaves.into_iter().map(|(index, name)| BranchRow::Branch {
            index,
            depth,
            label: name.to_owned(),
        }));
    }
}

/// 这些分支都在同一个子目录里时返回它的名字。
fn only_folder<'a>(items: &[(usize, &'a str)]) -> Option<&'a str> {
    let (first, _) = items.first()?.1.split_once('/')?;
    items
        .iter()
        .all(|(_, name)| {
            name.split_once('/')
                .is_some_and(|(folder, _)| folder == first)
        })
        .then_some(first)
}

impl App {
    /// 面板是否正在列出分支。
    pub(crate) fn picker_lists_branches(&self) -> bool {
        self.picker_branch_rows.is_some()
    }

    /// 打开分支选择器。
    pub(in crate::app) fn open_branch_picker(&mut self) {
        self.finish_name_input();
        self.picker_places = None;
        self.picker_repository_rows = None;
        self.picker_open = true;
        self.picker_doc = self.editor.active().map(|d| d.id);
        self.picker_input.set_text("");
        self.picker_branch_rows = Some(Vec::new());
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 关闭分支选择器；没有打开时不做什么。
    pub(in crate::app) fn close_branch_picker(&mut self) {
        if self.picker_lists_branches() {
            self.close_picker();
        }
    }

    /// 按查询重新排列各行，选中第一个分支（没有分支时是第一个能选中的行）。
    pub(in crate::app) fn branch_picker_refresh(&mut self) {
        let rows = self.current_branch_rows();
        self.picker_selected = rows
            .iter()
            .position(|row| matches!(row, BranchRow::Branch { .. }))
            .or_else(|| rows.iter().position(BranchRow::selectable))
            .unwrap_or(0);
        self.set_branch_rows(rows);
        self.picker_reveal();
    }

    /// 分支列表从 `before` 刷新以后重排：下标可能已经指向别的分支，选中项按名字留住。
    pub(in crate::app) fn branch_picker_refreshed(&mut self, before: &[Branch]) {
        if self.picker_lists_branches() {
            self.keep_branch_selection(before);
        }
    }

    /// 查询在名字 `text` 里命中的字符下标：绘制时标出来。
    pub(crate) fn branch_picker_matched(&self, text: &str) -> Vec<usize> {
        matched(&needle(self.picker_input.text()), text)
    }

    /// 重新排列各行，留住 `before` 列表下选中的那一项；它不在了时选中第一个能选中的行。
    fn keep_branch_selection(&mut self, before: &[Branch]) {
        let selected = self.selected_branch_row().cloned();
        let rows = self.current_branch_rows();
        let same = |row: &BranchRow| {
            selected
                .as_ref()
                .is_some_and(|s| s.same(row, before, &self.git.branches))
        };
        self.picker_selected = rows
            .iter()
            .position(same)
            .or_else(|| rows.iter().position(BranchRow::selectable))
            .unwrap_or(0);
        self.set_branch_rows(rows);
        self.relayout();
        self.request_redraw();
    }

    fn current_branch_rows(&self) -> Vec<BranchRow> {
        branch_rows(
            &self.git.branches,
            &self.git.recent_branches,
            &needle(self.picker_input.text()),
            &self.branch_folders,
        )
    }

    fn set_branch_rows(&mut self, rows: Vec<BranchRow>) {
        self.picker_hits = (0..rows.len()).collect();
        self.picker_branch_rows = Some(rows);
    }

    fn selected_branch_row(&self) -> Option<&BranchRow> {
        self.picker_branch_rows.as_ref()?.get(self.picker_selected)
    }

    /// 返回向下（`down`）或向上移动一步后选中的行，跳过分组标题；到达两端后不动。
    pub(in crate::app) fn branch_picker_step(&self, down: bool) -> usize {
        let rows = self.picker_branch_rows.as_deref().unwrap_or_default();
        let at = self.picker_selected;
        let next = if down {
            (at + 1..rows.len()).find(|&i| rows[i].selectable())
        } else {
            (0..at).rev().find(|&i| rows[i].selectable())
        };
        next.unwrap_or(at)
    }

    /// 确认选中的行：目录展开或收起，分支签出，其余打开菜单。
    pub(in crate::app) fn branch_picker_accept(&mut self) {
        match self.selected_branch_row().cloned() {
            Some(BranchRow::Actions) => self.open_branch_row_menu(MenuTarget::Branch),
            Some(BranchRow::Folder { remote, path, .. }) => self.toggle_branch_folder(remote, path),
            Some(BranchRow::Branch { index, .. }) => self.checkout_branch_row(index),
            Some(BranchRow::Heading(_)) | None => {}
        }
    }

    /// 签出第 `index` 个分支：当前分支只关闭选择器，不能签出的（工作树目录已不在）打开操作菜单，
    /// 灰掉的菜单项说明原因。
    fn checkout_branch_row(&mut self, index: usize) {
        let Some(branch) = self.git.branches.get(index).cloned() else {
            return;
        };
        if branch.current {
            self.close_picker();
        } else if self.git.stale_branches().contains(&local_name(&branch)) {
            self.open_branch_row_menu(MenuTarget::BranchOps(index));
        } else {
            self.close_picker();
            self.run_branch_op(BranchOp::Checkout, branch);
        }
    }

    fn toggle_branch_folder(&mut self, remote: bool, path: String) {
        let key = (remote, path);
        if !self.branch_folders.remove(&key) {
            self.branch_folders.insert(key);
        }
        let branches = self.git.branches.clone();
        self.keep_branch_selection(&branches);
        self.picker_reveal();
    }

    /// 处理没有查询时的左右键：右键展开目录、打开分支或「Git 操作」的菜单，左键收起目录、回到
    /// 上一级目录。返回是否已处理；有查询时左右键移动输入框的光标。
    pub(in crate::app) fn branch_picker_side(&mut self, right: bool) -> bool {
        if !self.picker_lists_branches() || !self.picker_input.text().is_empty() {
            return false;
        }
        let Some(row) = self.selected_branch_row().cloned() else {
            return false;
        };
        match (row, right) {
            (BranchRow::Actions, true) => self.open_branch_row_menu(MenuTarget::Branch),
            (BranchRow::Branch { index, .. }, true) => {
                self.open_branch_row_menu(MenuTarget::BranchOps(index));
            }
            (
                BranchRow::Folder {
                    remote, path, open, ..
                },
                _,
            ) if open != right => self.toggle_branch_folder(remote, path),
            (BranchRow::Folder { .. }, true) => {
                self.picker_selected = self.branch_picker_step(true);
                self.picker_reveal();
                self.request_redraw();
            }
            (_, false) => self.select_parent_branch_folder(),
            _ => {}
        }
        true
    }

    /// 选中当前行所在的目录；已经在最外层时不动。
    fn select_parent_branch_folder(&mut self) {
        let rows = self.picker_branch_rows.as_deref().unwrap_or_default();
        let depth_of = |row: &BranchRow| match row {
            BranchRow::Folder { depth, .. } | BranchRow::Branch { depth, .. } => Some(*depth),
            _ => None,
        };
        let Some(depth) = rows.get(self.picker_selected).and_then(depth_of) else {
            return;
        };
        let parent = (0..self.picker_selected).rev().find(|&i| {
            matches!(rows[i], BranchRow::Folder { .. }) && depth_of(&rows[i]) < Some(depth)
        });
        if let Some(parent) = parent {
            self.picker_selected = parent;
            self.picker_reveal();
            self.request_redraw();
        }
    }

    /// 点击第 `index` 行：点在「⋯」上打开操作菜单，否则选中并确认。
    pub(in crate::app) fn click_branch_row(&mut self, index: usize, x: f32) {
        let rows = self.picker_branch_rows.as_deref().unwrap_or_default();
        let Some(row) = rows.get(index).filter(|row| row.selectable()).cloned() else {
            return;
        };
        self.picker_selected = index;
        let parts = crate::view::picker_parts(self, self.layout.picker);
        let button = more_button(parts.row(index, self.picker_scroll));
        match row {
            BranchRow::Branch { index, .. } if x >= button.x && x <= button.right() => {
                self.open_branch_row_menu(MenuTarget::BranchOps(index));
            }
            _ => self.picker_accept(),
        }
        self.request_redraw();
    }

    /// 在选中行的右端弹出菜单 `target`。
    fn open_branch_row_menu(&mut self, target: MenuTarget) {
        let parts = crate::view::picker_parts(self, self.layout.picker);
        let row = parts.row(self.picker_selected, self.picker_scroll);
        self.open_context_menu(target, (row.right() - 14.0, row.bottom()));
    }
}

#[cfg(test)]
mod tests;
