//! 仓库选择器：提交视图里点项目名时在快速打开面板里列出目录下的仓库，可以搜索。
//!
//! 提交前须能确认并切换到正确的仓库，因此有多个仓库时项目名就是仓库的入口；第一行打开原来的项目
//! 菜单。每个仓库写着分支和改动数（[`App::git_repository_detail`]）。

use crate::app::input::context_menu::MenuTarget;
use crate::app::input::palette::{needle, rate};
use crate::app::{App, Focus};
use crate::layout::SidebarView;

/// 仓库选择器的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryRow {
    /// 打开项目菜单（最近的项目、工作树、打开和关闭目录）。
    ProjectMenu,
    /// [`GitState::repositories`](crate::app::git::GitState::repositories) 里的一个仓库。
    Repository(usize),
}

impl App {
    /// 面板是否正在列出仓库。
    pub(crate) fn picker_lists_repositories(&self) -> bool {
        self.picker_repository_rows.is_some()
    }

    /// 点击项目名是否打开仓库选择器：提交视图里有多个仓库时。
    pub(in crate::app) fn project_name_picks_repository(&self) -> bool {
        self.sidebar_view == SidebarView::Commit && self.git.repositories.len() >= 2
    }

    /// 打开仓库选择器，选中当前仓库，并在后台重新查找一次。
    pub(in crate::app) fn open_repository_picker(&mut self) {
        self.git_discover();
        self.git_scan_repositories();
        self.finish_name_input();
        self.picker_places = None;
        self.picker_branch_rows = None;
        self.picker_open = true;
        self.picker_doc = self.editor.active().map(|d| d.id);
        self.picker_input.set_text("");
        self.picker_repository_rows = Some(Vec::new());
        self.focus = Focus::Picker;
        self.picker_refresh();
        self.relayout();
        self.reset_blink();
        self.request_redraw();
    }

    /// 关闭仓库选择器；没有打开时不做什么。
    pub(in crate::app) fn close_repository_picker(&mut self) {
        if self.picker_lists_repositories() {
            self.close_picker();
        }
    }

    /// 按查询重新排列各行，选中当前仓库（它不在结果里时是第一个仓库）。
    pub(in crate::app) fn repository_picker_refresh(&mut self) {
        let query = needle(self.picker_input.text());
        let mut rows = Vec::new();
        if query.is_empty() {
            rows.push(RepositoryRow::ProjectMenu);
        }
        for index in 0..self.git.repositories.len() {
            if query.is_empty() || rate(&query, &self.git_repository_name(index)).is_some() {
                rows.push(RepositoryRow::Repository(index));
            }
        }
        let current = rows.iter().position(|row| match row {
            RepositoryRow::Repository(index) => {
                self.git.repositories.get(*index) == self.git.root.as_ref()
            }
            RepositoryRow::ProjectMenu => false,
        });
        let first = rows
            .iter()
            .position(|row| matches!(row, RepositoryRow::Repository(_)));
        self.picker_selected = current.or(first).unwrap_or(0);
        self.picker_hits = (0..rows.len()).collect();
        self.picker_repository_rows = Some(rows);
    }

    /// 后台找到了新的仓库：重新排列；选中项回到当前仓库。
    pub(in crate::app) fn repository_picker_refreshed(&mut self) {
        if self.picker_lists_repositories() {
            self.repository_picker_refresh();
            self.relayout();
        }
    }

    /// 确认选中的行：切换到仓库，或者打开项目菜单。
    pub(in crate::app) fn repository_picker_accept(&mut self) {
        let row = self
            .picker_repository_rows
            .as_ref()
            .and_then(|rows| rows.get(self.picker_selected).copied());
        match row {
            Some(RepositoryRow::ProjectMenu) => {
                let parts = crate::view::picker_parts(self, self.layout.picker);
                let row = parts.row(self.picker_selected, self.picker_scroll);
                self.open_context_menu(MenuTarget::Project, (row.x + 14.0, row.bottom()));
            }
            Some(RepositoryRow::Repository(index)) => {
                self.close_picker();
                self.git_switch_repository(index);
            }
            None => {}
        }
        self.request_redraw();
    }

    /// 查询在名字 `text` 里命中的字符下标：绘制时标出来。
    pub(crate) fn repository_picker_matched(&self, text: &str) -> Vec<usize> {
        crate::app::input::palette::matched(&needle(self.picker_input.text()), text)
    }
}
