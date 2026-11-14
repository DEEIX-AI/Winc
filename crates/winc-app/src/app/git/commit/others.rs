//! 「显示全部仓库」：当前仓库的更改之后，列出打开的目录中其他仓库的更改。
//!
//! 其他仓库的各行只读：点击标题行或其中的文件即切换到该仓库；勾选、回滚和提交只在当前仓库中
//! 进行。各行的内容取自后台读到的各仓库状态（`GitState::summaries`）。

use std::sync::PoisonError;

use super::*;

/// 排在当前仓库的更改之后的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OtherRow {
    /// 当前仓库没有更改时排在最前的一行说明。
    Clean,
    /// 另一个仓库的标题行：该仓库在仓库列表（`GitState::repositories`）中的下标。
    Repository(usize),
    /// 另一个仓库中有更改的文件：仓库的下标，文件在该仓库状态各项中的下标。
    File(usize, usize),
}

impl App {
    /// 「显示全部仓库」是否开着：没有选过时勾选式开着，暂存区视图关着。
    pub(crate) fn commit_all_repositories(&self) -> bool {
        let chosen = self.state.commit_all_repositories;
        chosen.unwrap_or(!self.git.commit.staging())
    }

    /// 是否在当前仓库的更改之后列出其他仓库：开着「显示全部仓库」，打开的目录中有多个仓库，
    /// 并且不在浏览历史提交的文件。
    pub(crate) fn commit_shows_others(&self) -> bool {
        self.commit_all_repositories()
            && !self.git.commit.browse
            && self.git.repositories.len() >= 2
    }

    /// 返回排在当前仓库的更改之后的各行；不列出其他仓库时为空。
    ///
    /// 每个其他仓库一行标题，其下是它有更改的文件；状态尚未读到的仓库只有标题行。
    pub(crate) fn commit_other_rows(&self) -> Vec<OtherRow> {
        if !self.commit_shows_others() {
            return Vec::new();
        }
        let root = self.git.root.as_deref();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let mut rows = Vec::new();
        for (i, repo) in self.git.repositories.iter().enumerate() {
            if Some(repo.as_path()) == root {
                continue;
            }
            rows.push(OtherRow::Repository(i));
            let files = summaries.get(repo).map_or(0, |s| s.entries.len());
            rows.extend((0..files).map(|entry| OtherRow::File(i, entry)));
        }
        let clean = (self.git.status.as_ref()).is_some_and(|s| s.entries.is_empty());
        if clean && !rows.is_empty() {
            rows.insert(0, OtherRow::Clean);
        }
        rows
    }

    /// 用第 `repo` 个仓库的根目录和读到的状态调用 `read`；该仓库的状态尚未读到时为 `None`。
    pub(crate) fn other_status<R>(
        &self,
        repo: usize,
        read: impl FnOnce(&Path, &Status) -> R,
    ) -> Option<R> {
        let root = self.git.repositories.get(repo)?;
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        summaries.get(root).map(|status| read(root, status))
    }

    /// 打开或关闭「显示全部仓库」；所有窗口共用，下次启动沿用。
    pub(in crate::app) fn toggle_all_repositories(&mut self) {
        let on = !self.commit_all_repositories();
        self.update_state(|state| state.commit_all_repositories = Some(on));
        if on {
            self.git_read_summaries();
        }
        self.git.commit.scroll = self.git.commit.scroll.min(self.commit_max_scroll());
        self.request_redraw();
    }

    /// 点了其他仓库的标题行或文件：切换到该仓库，点的是文件时选中它并预览差异。
    pub(super) fn press_other(&mut self, target: CommitTarget) {
        let (repo, entry) = match target {
            CommitTarget::OtherRepository(repo) => (repo, None),
            CommitTarget::OtherFile(repo, entry) => (repo, Some(entry)),
            _ => return,
        };
        let path = entry.and_then(|entry| {
            let path = |status: &Status| status.entries.get(entry).map(|e| e.path.clone());
            self.other_status(repo, |_, status| path(status)).flatten()
        });
        self.git_switch_repository(repo);
        self.focus = Focus::CommitList;
        if let Some(path) = path {
            self.select_commit_file(path);
        }
        self.request_redraw();
    }

    /// 其他仓库中文件 `path` 是否勾上了。
    pub(crate) fn other_checked(&self, path: &Path) -> bool {
        self.git.commit.other_checked.contains(path)
    }

    /// 第 `repo` 个仓库标题行的勾选框：全部勾上为 `Some(true)`，勾了一部分为 `Some(false)`，
    /// 一个都没勾或没有文件时为 `None`。
    pub(crate) fn other_group_state(&self, repo: usize) -> Option<bool> {
        let checked = &self.git.commit.other_checked;
        let count = |status: &Status| {
            let on = status.entries.iter().filter(|e| checked.contains(&e.path));
            (on.count(), status.entries.len())
        };
        match self.other_status(repo, |_, status| count(status))? {
            (0, _) => None,
            (on, all) => Some(on == all),
        }
    }

    /// 其他仓库中勾上并且仍然有更改的文件数。
    pub(crate) fn other_checked_count(&self) -> usize {
        if self.git.commit.other_checked.is_empty() || !self.commit_shows_others() {
            return 0;
        }
        let root = self.git.root.as_deref();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let others = (self.git.repositories.iter()).filter(|repo| Some(repo.as_path()) != root);
        let entries = others
            .filter_map(|repo| summaries.get(repo))
            .flat_map(|s| &s.entries);
        entries.filter(|e| self.other_checked(&e.path)).count()
    }

    /// 其他仓库的状态重读以后，去掉已经没有更改的文件的勾选。
    pub(in crate::app) fn prune_other_checked(&mut self) {
        if self.git.commit.other_checked.is_empty() {
            return;
        }
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let changed = |path: &PathBuf| {
            summaries
                .values()
                .any(|s| s.entries.iter().any(|e| e.path == *path))
        };
        self.git.commit.other_checked.retain(changed);
    }

    /// 点了其他仓库的勾选框：文件的勾选框只管这一个文件；标题行的勾选框在全部勾上时取消全部，
    /// 否则全部勾上。
    pub(super) fn press_other_check(&mut self, target: CommitTarget) {
        let paths = |status: &Status, entry: Option<usize>| -> Vec<PathBuf> {
            let entries = status.entries.iter().enumerate();
            let wanted = entries.filter(|(i, _)| entry.is_none_or(|entry| entry == *i));
            wanted.map(|(_, e)| e.path.clone()).collect()
        };
        let (repo, entry) = match target {
            CommitTarget::OtherGroupCheck(repo) => (repo, None),
            CommitTarget::OtherCheck(repo, entry) => (repo, Some(entry)),
            _ => return,
        };
        let Some(paths) = self.other_status(repo, |_, status| paths(status, entry)) else {
            return;
        };
        let checked = &mut self.git.commit.other_checked;
        if paths.iter().all(|path| checked.contains(path)) {
            checked.retain(|path| !paths.contains(path));
        } else {
            checked.extend(paths);
        }
        self.request_redraw();
    }
}
