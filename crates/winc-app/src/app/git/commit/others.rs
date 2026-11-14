//! 「显示全部仓库」：当前仓库的更改之后，列出打开的目录中其他仓库的更改。
//!
//! 点击其他仓库的标题行或其中的文件即切换到该仓库。勾选式下各行带勾选框，勾上的文件随当前
//! 仓库一起提交；暂存区视图中各仓库按「已暂存的更改」「更改」分组，可以暂存、取消暂存，
//! 已暂存的内容随当前仓库一起提交。各行的内容取自后台读到的各仓库状态。

use std::sync::PoisonError;

use super::*;

/// 排在当前仓库的更改之后的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OtherRow {
    /// 当前仓库没有更改时排在最前的一行说明。
    Clean,
    /// 另一个仓库的标题行：该仓库在仓库列表（`GitState::repositories`）中的下标。
    Repository(usize),
    /// 暂存区视图中另一个仓库的分组行：仓库的下标，是否为「已暂存的更改」一组。
    Group(usize, bool),
    /// 暂存区视图中另一个仓库已暂存的文件：仓库的下标，文件在该仓库状态各项中的下标。
    Staged(usize, usize),
    /// 另一个仓库中有更改的文件：仓库的下标，文件在该仓库状态各项中的下标。暂存区视图中是
    /// 尚未暂存的那一部分。
    File(usize, usize),
}

/// `entry` 暂存的内容是否随提交写入：暂存区中有内容，并且不在冲突中。
fn commits_staged(entry: &Entry) -> bool {
    entry.staged.is_some() && !entry.conflicted()
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
    /// 每个其他仓库一行标题，其下是它有更改的文件；状态尚未读到的仓库只有标题行。暂存区视图中
    /// 文件分为已暂存和未暂存两组，各有一行分组标题，两部分都有内容的文件在两组中各列一次。
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
            let Some(status) = summaries.get(repo) else {
                continue;
            };
            if !self.git.commit.staging() {
                rows.extend((0..status.entries.len()).map(|entry| OtherRow::File(i, entry)));
                continue;
            }
            let entries = || status.entries.iter().enumerate();
            let staged = entries().filter(|(_, e)| commits_staged(e));
            let staged: Vec<_> = staged.map(|(n, _)| OtherRow::Staged(i, n)).collect();
            let rest = entries().filter(|(_, e)| e.unstaged.is_some() || !commits_staged(e));
            let rest: Vec<_> = rest.map(|(n, _)| OtherRow::File(i, n)).collect();
            for (group, files) in [(true, staged), (false, rest)] {
                if !files.is_empty() {
                    rows.push(OtherRow::Group(i, group));
                    rows.extend(files);
                }
            }
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
            CommitTarget::OtherFile(repo, entry) | CommitTarget::OtherStagedFile(repo, entry) => {
                (repo, Some(entry))
            }
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

    /// 暂存区视图中各个其他仓库里随当前仓库一并提交的文件数：仓库的根目录和已暂存的文件数。
    ///
    /// 不列出其他仓库时为空；有合并、变基等操作进行中的仓库不在这里提交。
    fn other_staged(&self) -> Vec<(PathBuf, usize)> {
        if !self.git.commit.staging() || !self.commit_shows_others() {
            return Vec::new();
        }
        let root = self.git.root.as_deref();
        let summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
        let others = (self.git.repositories.iter()).filter(|repo| Some(repo.as_path()) != root);
        let counted = others.filter_map(|repo| {
            let status = summaries.get(repo).filter(|s| s.operation.is_none())?;
            let count = status.entries.iter().filter(|e| commits_staged(e)).count();
            (count > 0).then(|| (repo.clone(), count))
        });
        counted.collect()
    }

    /// 暂存区视图中其他仓库里随当前仓库一并提交的文件数。
    pub(crate) fn other_staged_count(&self) -> usize {
        self.other_staged().iter().map(|(_, count)| count).sum()
    }

    /// 暂存区视图中有内容随当前仓库一并提交的其他仓库：根目录，是否附加签署行。
    pub(super) fn other_staged_roots(&self) -> Vec<(PathBuf, bool)> {
        let roots = self.other_staged().into_iter();
        roots
            .map(|(root, _)| {
                let sign_off = self.state.sign_off_repos.contains(&root);
                (root, sign_off)
            })
            .collect()
    }

    /// 当前按下「提交」不会提交的原因（见 [`CommitState::blocker`]），计入其他仓库暂存的内容。
    pub(crate) fn commit_blocker(&self, status: &Status) -> Option<String> {
        let others = self.other_staged_count();
        self.git.commit.blocker(status, others)
    }

    /// 点了其他仓库文件行或分组行行尾的「＋」「−」：界面立即变化，命令在后台按顺序执行。
    ///
    /// 冲突中的文件不在这里处理：切换到该仓库解决后再暂存。
    pub(super) fn press_other_stage(&mut self, target: CommitTarget) {
        let (repo, entry, on) = match target {
            CommitTarget::OtherStage(repo, entry, on) => (repo, Some(entry), on),
            CommitTarget::OtherGroupStage(repo, on) => (repo, None, on),
            _ => return,
        };
        let Some(root) = self.git.repositories.get(repo).cloned() else {
            return;
        };
        let mut touched = Vec::new();
        {
            let mut summaries = (self.git.summaries.lock()).unwrap_or_else(PoisonError::into_inner);
            let Some(status) = summaries.get_mut(&root) else {
                return;
            };
            let entries = status.entries.iter_mut().enumerate();
            let wanted = entries.filter(|(i, e)| entry.is_none_or(|n| n == *i) && !e.conflicted());
            for (_, e) in wanted {
                staging::flip_staged(e, on, &mut touched);
            }
        }
        if touched.is_empty() {
            return;
        }
        (self.git.commit.other_stage_queue).push_back((root, on, touched));
        self.pump_other_staging();
        self.request_redraw();
    }

    /// 队列中有等待的操作、且没有操作正在执行时，在后台执行下一步。
    fn pump_other_staging(&mut self) {
        if self.git.commit.other_stage_running || self.git.root.is_none() {
            return;
        }
        let Some((other, on, paths)) = self.git.commit.other_stage_queue.pop_front() else {
            return;
        };
        self.git.commit.other_stage_running = true;
        let sender = self.git.commit.results.0.clone();
        self.run_git_job(move |git, _| {
            let result = git.and_then(|git| match on {
                true => git.stage(&other, &paths).map(drop),
                false => git.unstage(&other, &paths),
            });
            let _ = sender.send(Outcome::OtherStaged(result));
        });
    }

    /// 写入其他仓库暂存区的一步操作结束：继续下一步；全部完成后重读各仓库的状态，并执行
    /// 等待中的提交。
    pub(super) fn other_staging_finished(&mut self, result: Result<(), String>) {
        self.git.commit.other_stage_running = false;
        if let Err(e) = result {
            // 界面已按成功的结果显示：放弃其余的操作，按各仓库的实际状态重新显示。
            self.git.commit.other_stage_queue.clear();
            self.git.commit.commit_queued = false;
            self.notify(t!("git-stage-failed", error = e));
        }
        if !self.git.commit.other_stage_queue.is_empty() {
            self.pump_other_staging();
            return;
        }
        self.git_read_summaries();
        if std::mem::take(&mut self.git.commit.commit_queued) {
            self.git_commit();
        }
    }
}
