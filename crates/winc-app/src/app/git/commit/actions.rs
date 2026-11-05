//! 提交和回滚：先保存要提交的文件、请用户确认回滚，在后台运行 git 并收下结果。

use winc_editor::git::Synced;
use winc_i18n::t;

use super::*;
use crate::app::AlertButton;
use crate::app::git::branch::Then;

/// 提交成功后按钮显示「✓ 已提交」的时长（经验值）。
///
/// 短于状态栏提示的显示时长，足以让用户确认提交已完成。
const COMMITTED_SHOWN: Duration = Duration::from_millis(1600);

impl App {
    /// 返回「提交」按钮用于同步时与上游相差的（领先，落后）提交数；不用于同步时为 `None`。
    ///
    /// 没有改动、当前分支有上游、与上游有差异，且不在修改上一次提交或合并、变基等操作中时，
    /// 按钮用于同步。
    pub(crate) fn commit_sync(&self) -> Option<(u32, u32)> {
        let status = self.git.status.as_ref()?;
        let idle = !self.commit.busy && !self.commit.amend && status.operation.is_none();
        let differs = status.ahead > 0 || status.behind > 0;
        (idle && status.entries.is_empty() && differs && self.has_upstream())
            .then_some((status.ahead, status.behind))
    }

    /// 是否正在后台与上游同步（[`App::git_sync`]）。
    pub(crate) fn commit_syncing(&self) -> bool {
        self.branch.running.as_deref() == Some(t!("git-op-sync").as_ref() as &str)
    }

    /// 与上游同步：先询问（配置项 `git.confirmSync` 为 `false` 时不问），确认后开始。
    pub(in crate::app) fn git_sync(&mut self) {
        let Some((ahead, behind)) = self.commit_sync() else {
            return;
        };
        if self.config.confirm_sync != Some(false) && self.window.is_some() {
            let detail = match (ahead, behind) {
                (_, 0) => t!("git-sync-confirm-push", ahead = ahead),
                (0, _) => t!("git-sync-confirm-pull", behind = behind),
                _ => t!("git-sync-confirm-both", ahead = ahead, behind = behind),
            };
            let buttons = vec![
                AlertButton::new(t!("git-sync-confirm-button")),
                AlertButton::cancel(t!("common-cancel")),
            ];
            let asked = Some((t!("git-confirm-dont-ask"), false));
            let message = t!("git-sync-confirm");
            self.show_alert(message, &detail, buttons, asked, |index, never| {
                (index == 0).then_some(UserEvent::GitSync(never))
            });
            return;
        }
        self.run_git_sync(false);
    }

    /// 同步的提示框按了「同步」：`never_ask` 表示勾选了「不再询问」。
    pub(in crate::app) fn confirm_git_sync(&mut self, never_ask: bool) {
        self.never_ask_sync(never_ask);
        self.run_git_sync(false);
    }

    /// 勾选了「不再询问」时写入配置，之后同步、提交并推送都不再询问。
    fn never_ask_sync(&mut self, never_ask: bool) {
        if never_ask {
            self.config.confirm_sync = Some(false);
            self.save_config();
        }
    }

    /// 在后台同步（[`winc_editor::git::Git::sync`]）；需要强制推送而 `force` 为假时转为询问。
    pub(in crate::app) fn run_git_sync(&mut self, force: bool) {
        self.git_operate_then(t!("git-op-sync").into(), move |git, root| {
            git.sync(root, force).map(|synced| match synced {
                Synced::Done(summary) => (summary, Then::Nothing),
                Synced::NeedsForce(count) => (String::new(), Then::ConfirmForcePush(count)),
            })
        });
    }

    /// 同步需要强制推送：无论配置如何都询问，确认后才替换上游的 `count` 个提交。
    ///
    /// 没有窗口（测试中）时无法确认，在状态栏说明，不推送。
    pub(in crate::app) fn confirm_force_push(&mut self, count: usize) {
        let message = t!("git-force-push-confirm");
        if self.window.is_some() {
            let detail = t!("git-force-push-confirm-detail", count = count);
            let button = t!("git-force-push-confirm-button");
            self.show_confirm(message, &detail, button, |ok| {
                ok.then_some(UserEvent::GitForcePush)
            });
            return;
        }
        self.flash(message);
    }

    /// 提交并推送：可以提交时先询问（配置项 `git.confirmSync` 为 `false` 时不问）。
    pub(in crate::app) fn ask_commit_and_push(&mut self) {
        let ready = !self.commit.busy
            && self
                .git
                .status
                .as_ref()
                .is_some_and(|status| self.commit.blocker(status).is_none());
        if ready && self.config.confirm_sync != Some(false) && self.window.is_some() {
            let detail = match self.commit_scope() {
                Some((count, Some(branch))) => {
                    t!(
                        "git-commit-push-confirm-scope",
                        count = count,
                        branch = branch
                    )
                }
                _ => t!("git-commit-push-confirm-detail").to_owned(),
            };
            let buttons = vec![
                AlertButton::new(t!("git-commit-push-confirm-button")),
                AlertButton::cancel(t!("common-cancel")),
            ];
            let asked = Some((t!("git-confirm-dont-ask"), false));
            let message = t!("git-commit-push-confirm");
            self.show_alert(message, &detail, buttons, asked, |index, never| {
                (index == 0).then_some(UserEvent::GitCommitAndPush(never))
            });
            return;
        }
        // 不能提交时由提交的入口说明原因。
        self.git_commit_and_push();
    }

    /// 提交并推送的提示框按了确认：`never_ask` 表示勾选了「不再询问」。
    pub(in crate::app) fn confirm_commit_and_push(&mut self, never_ask: bool) {
        self.never_ask_sync(never_ask);
        self.git_commit_and_push();
    }

    /// 提交勾上的文件。
    ///
    /// 没写提交信息、什么都没勾时在状态栏说明，不提交。
    pub(in crate::app) fn git_commit(&mut self) {
        if self.commit.busy {
            return;
        }
        let message = self.commit.message.text().trim().to_owned();
        let rebasing = self
            .git
            .status
            .as_ref()
            .is_some_and(|s| s.operation == Some(Operation::Rebase));
        if message.is_empty() && !rebasing {
            self.flash(t!("git-commit-message-required"));
            self.focus = Focus::CommitMessage;
            return;
        }
        // 提交按磁盘上的内容：勾上的文件在编辑器里有没保存的修改就先保存（只提交一部分改动的
        // 文件也按保存以后的内容拼），提交的就是用户看到的样子。
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let checked: Vec<PathBuf> = self
            .commit
            .checked_entries(status)
            .map(|e| e.path.clone())
            .collect();
        if !self.save_dirty_documents(&checked) {
            return;
        }
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        // 勾选的冲突文件中仍有冲突标记时拒绝提交，否则 `git add` 将其视为已解决，标记和两侧内容会
        // 一并进入提交；这里读取刚保存的磁盘内容，使编辑器刚移除标记、状态尚未刷新时的结果也正确。
        let unresolved = self
            .commit
            .checked_entries(status)
            .filter(|e| e.conflicted() && winc_editor::git::merge::file_has_conflicts(&e.path))
            .count();
        if unresolved > 0 {
            self.flash(t!("git-commit-unresolved-conflicts", count = unresolved));
            return;
        }
        let mut paths = Vec::new();
        let mut add = Vec::new();
        for entry in self.commit.checked_entries(status) {
            paths.push(entry.path.clone());
            // 改名的文件，原来的路径也要一起提交，否则提交里会同时留着新旧两个文件。
            paths.extend(entry.from.clone());
            if entry.untracked() {
                add.push(entry.path.clone());
            }
        }
        if paths.is_empty() && !self.commit.amend && !rebasing {
            self.flash(t!("git-commit-nothing-checked"));
            return;
        }
        // 只提交一部分改动的文件：按磁盘上的内容拼出要提交的内容。
        let parts: Vec<(PathBuf, Vec<Skip>)> = self
            .commit
            .checked_entries(status)
            .filter_map(|e| {
                let skipped = self.commit.skipped.get(&e.path)?;
                (!skipped.is_empty()).then(|| (e.path.clone(), skipped.clone()))
            })
            .collect();
        // 只暂存了一部分的文件整个勾着时只提交已暂存的内容（在终端里用 `git add -p` 选好的），
        // 其余改动留在工作区；在差异里勾掉过几处时按勾选提交（即 `parts`）。合并、拣选、
        // 还原进行中提交整个暂存区，不分开处理。
        let staged: Vec<PathBuf> = self
            .commit
            .checked_entries(status)
            .filter(|e| e.partly_staged() && !self.commit.skips_hunks(e))
            .filter(|_| status.operation.is_none())
            .map(|e| e.path.clone())
            .collect();
        // 变基停下来时：勾上的文件标记为已解决，继续变基（git 用原来的提交信息）。
        if rebasing {
            self.git_operate(t!("git-op-continue-rebase").to_owned(), move |git, root| {
                if !paths.is_empty() {
                    let mut args = vec!["add".to_owned(), "--all".to_owned(), "--".to_owned()];
                    args.extend(paths.iter().map(|p| {
                        p.strip_prefix(root)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .into_owned()
                    }));
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    git.operate(root, &args)?;
                }
                git.continue_operation(root, Operation::Rebase)
            });
            return;
        }
        let amend = self.commit.amend;
        let sender = self.commit.results.0.clone();
        self.commit.busy = true;
        self.run_git_job(move |git, root| {
            let result = git.and_then(|git| {
                let mut partial = parts
                    .iter()
                    .map(|(path, skipped)| partial_content(git, root, path, skipped))
                    .collect::<Result<Vec<_>, _>>()?;
                for path in &staged {
                    partial.push(staged_content(git, root, path)?);
                }
                git.commit(root, &message, &paths, &add, &partial, amend)
            });
            let _ = sender.send(Outcome::Committed(result, message, root.to_path_buf()));
        });
    }

    /// 保存 `paths` 里在编辑器中开着、还没保存的文档，返回是否全部保存成功。
    ///
    /// 有一个保存失败就停下；失败的原因保存时已经说明了。
    fn save_dirty_documents(&mut self, paths: &[PathBuf]) -> bool {
        for path in paths {
            let canonical = winc_editor::canonical_path(path);
            let dirty = self
                .editor
                .documents
                .iter()
                .find(|d| d.path.as_deref() == Some(canonical.as_path()) && d.is_dirty())
                .map(|d| d.id);
            if let Some(id) = dirty
                && !self.write_document(id, None, true)
            {
                return false;
            }
        }
        true
    }

    /// 确认后中止进行中的合并、变基等操作。
    ///
    /// 中止会丢弃已部分解决的冲突，因此须先确认。
    pub(super) fn confirm_abort(&mut self) {
        let Some(operation) = self.git.status.as_ref().and_then(|s| s.operation) else {
            return;
        };
        let label = operation.label();
        let message = t!("git-abort-confirm", operation = label);
        if self.window.is_some() {
            let detail = t!("git-abort-confirm-detail");
            let button = t!("git-abort-confirm-button", operation = label);
            self.show_confirm(&message, detail, &button, move |ok| {
                ok.then_some(UserEvent::GitAbort)
            });
            return;
        }
        // 没有窗口（测试中）时直接中止。
        self.run_branch_command(crate::app::input::context_menu::MenuCommand::GitAbort);
    }

    /// 回滚前先确认（系统确认框）；确认后回滚勾上的文件。
    pub(super) fn ask_rollback(&mut self) {
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let paths: Vec<PathBuf> = self
            .commit
            .checked_entries(status)
            .map(|e| e.path.clone())
            .collect();
        if paths.is_empty() {
            self.flash(t!("git-rollback-nothing-checked"));
            return;
        }
        self.confirm_rollback(paths);
    }

    /// 先请用户确认，确认后回滚 `paths`。
    pub(super) fn confirm_rollback(&mut self, paths: Vec<PathBuf>) {
        let message = match paths.as_slice() {
            [one] => {
                let name = winc_editor::display_name(one);
                t!("git-rollback-confirm-one", name = name)
            }
            many => t!("git-rollback-confirm-many", count = many.len()),
        };
        if self.window.is_some() {
            let detail = t!("git-rollback-confirm-detail");
            let button = t!("git-rollback-confirm-button");
            self.show_confirm(&message, detail, button, move |ok| {
                ok.then(|| UserEvent::GitRollback(paths.clone()))
            });
            return;
        }
        // 没有窗口（测试中）时直接回滚。
        self.git_rollback(paths);
    }

    /// 回滚 `paths`：已跟踪的恢复成 `HEAD` 里的内容，新加入版本管理的从暂存区移除并移到废纸篓，
    /// 未跟踪的移到废纸篓。
    ///
    /// 打开着的文档由目录监视仍然重新读取。
    pub(in crate::app) fn git_rollback(&mut self, paths: Vec<PathBuf>) {
        let Some(status) = self.git.status.as_ref() else {
            return;
        };
        let mut restore = Vec::new();
        let mut added = Vec::new();
        let mut trash = Vec::new();
        for entry in status.entries.iter().filter(|e| paths.contains(&e.path)) {
            if entry.untracked() {
                trash.push(entry.path.clone());
            } else if entry.staged == Some(winc_editor::git::Change::Added) {
                added.push(entry.path.clone());
                trash.push(entry.path.clone());
            } else {
                restore.push(entry.path.clone());
                restore.extend(entry.from.clone());
            }
        }
        let count = paths.len();
        // 移到废纸篓在界面线程上做（和文件树的删除同一个入口），git 在后台。
        for path in &trash {
            if let Err(e) = (self.trash)(path) {
                let (name, error) = (winc_editor::display_name(path), e.to_string());
                self.notify(t!("git-rollback-file-failed", name = name, error = error));
            }
        }
        if restore.is_empty() && added.is_empty() {
            self.flash(t!("git-rollback-done", count = count));
            self.git_refresh();
            return;
        }
        let sender = self.commit.results.0.clone();
        self.commit.busy = true;
        self.run_git_job(move |git, root| {
            let result = git
                .and_then(|git| git.rollback(root, &restore, &added))
                .map(|()| count);
            let _ = sender.send(Outcome::RolledBack(result, root.to_path_buf()));
        });
    }

    /// 收下后台交回来的提交、回滚结果和上一次提交的信息。
    pub(in crate::app) fn poll_commit(&mut self) {
        while let Ok(outcome) = self.commit.results.1.try_recv() {
            match outcome {
                Outcome::LastMessage(message) => {
                    // 等结果的时候用户已经关闭了「修改上一次提交」，就不填。
                    if self.commit.amend {
                        self.commit.message.set_text(&message);
                        self.commit.message.set_caret(message.len(), false);
                    }
                }
                Outcome::Committed(result, message, root) => {
                    // 提交的时候已经换到了别的仓库：只说一声结果，不推送（会推到现在这个仓库）、
                    // 也不动现在这个仓库的提交视图（换的时候已经清过，见 `reset_commit_view`）。
                    if self.git.root.as_deref() != Some(root.as_path()) {
                        let name = winc_editor::display_name(&root);
                        match result {
                            Ok(oid) => self.flash(t!("git-commit-done-in", name = name, oid = oid)),
                            Err(e) => {
                                self.notify(t!("git-commit-failed-in", name = name, error = e))
                            }
                        }
                        continue;
                    }
                    self.commit.busy = false;
                    match result {
                        Ok(oid) => {
                            // 勾上的都提交了；按块勾掉的那些改动还在工作区里，下次重新勾。
                            self.commit.skipped.clear();
                            // 提交期间用户又改了信息，就留着改过的，不清空。
                            if self.commit.message.text().trim() == message {
                                self.commit.message.remember();
                                self.commit.message.set_text("");
                            }
                            let subject = message.lines().next().unwrap_or_default();
                            if std::mem::take(&mut self.commit.push_after) {
                                self.run_branch_command(
                                    crate::app::input::context_menu::MenuCommand::GitPush,
                                );
                            }
                            self.commit.amend = false;
                            self.commit.draft = None;
                            self.commit.reset_checks();
                            self.flash(t!("git-commit-done", oid = &oid, subject = subject));
                            let short: String = oid.chars().take(7).collect();
                            self.commit.committed = Some((short, Instant::now() + COMMITTED_SHOWN));
                        }
                        Err(e) => {
                            self.commit.push_after = false;
                            self.notify(t!("git-commit-failed", error = e));
                        }
                    }
                    self.git_refresh();
                }
                Outcome::Last(last, root) => {
                    let oid = self.git.status.as_ref().and_then(|s| s.oid.as_ref());
                    if self.git.root.as_deref() == Some(root.as_path())
                        && last.as_ref().map(|l| &l.oid) == oid
                    {
                        self.commit.last = last;
                    }
                }
                Outcome::Undone(result, last, root) => {
                    let name = winc_editor::display_name(&root);
                    let here = self.git.root.as_deref() == Some(root.as_path());
                    let tag: String = last.oid.chars().take(7).collect();
                    match result {
                        Ok(()) if here => {
                            if self.commit.message.text().trim().is_empty() {
                                self.commit.message.set_text(&last.message);
                                let end = last.message.len();
                                self.commit.message.set_caret(end, false);
                            }
                            self.flash(t!("git-undo-commit-done", oid = tag));
                        }
                        Ok(()) => self.flash(t!("git-undo-commit-done-in", name = name, oid = tag)),
                        Err(e) => {
                            self.commit.last_for = None;
                            self.notify(t!("git-undo-commit-failed", error = e));
                        }
                    }
                    if here {
                        self.git_refresh();
                    }
                }
                Outcome::RolledBack(result, root) => {
                    if self.git.root.as_deref() != Some(root.as_path()) {
                        let name = winc_editor::display_name(&root);
                        match result {
                            Ok(count) => {
                                self.flash(t!("git-rollback-done-in", name = name, count = count))
                            }
                            Err(e) => {
                                self.notify(t!("git-rollback-failed-in", name = name, error = e))
                            }
                        }
                        continue;
                    }
                    self.commit.busy = false;
                    match result {
                        Ok(count) => self.flash(t!("git-rollback-done", count = count)),
                        Err(e) => self.notify(t!("git-rollback-failed", error = e)),
                    }
                    // 重新读打开着的文档，顺带刷新仓库状态。
                    self.check_disk();
                }
            }
        }
        self.request_redraw();
    }

    /// 换了仓库：提交视图回到初始的样子（写了一半的信息、「修改上一次提交」、勾选、正在提交都是
    /// 旧仓库的）；列表的显示方式（平铺、收起的分组）保留，旧仓库尚未交回的结果仍然收下。
    pub(in crate::app::git) fn reset_commit_view(&mut self) {
        let old = std::mem::take(&mut self.commit);
        self.commit.results = old.results;
        self.commit.flat = old.flat;
        self.commit.collapsed = old.collapsed;
    }

    /// 在后台对打开的仓库做一件提交视图的事（提交、回滚），做完以 [`UserEvent::GitCommit`] 收
    /// 结果（[`App::spawn_git`]）；找不到 git 时 `work` 收到原因。
    pub(super) fn run_git_job(
        &mut self,
        work: impl FnOnce(Result<&Git, String>, &Path) + Send + 'static,
    ) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        self.spawn_git(UserEvent::GitCommit, move |tool| {
            work(tool.as_ref().map_err(unusable), &root);
        });
    }
}

/// 文件 `path` 只提交勾掉的 `skipped` 以外的改动时要提交成的内容：`HEAD` 里的版本用上磁盘上
/// 其余的改动。
fn partial_content(
    git: &Git,
    root: &Path,
    path: &Path,
    skipped: &[Skip],
) -> Result<Partial, String> {
    let name = winc_editor::display_name(path).into_owned();
    let head = git
        .head_content(root, path, &winc_editor::format::Cancel::default())?
        .unwrap_or_default();
    let disk = std::fs::read(path)
        .map_err(|e| t!("git-read-file-failed", name = &name, error = e.to_string()))?;
    let (Ok(head), Ok(disk)) = (String::from_utf8(head), String::from_utf8(disk)) else {
        return Err(t!("git-commit-partial-not-utf8", name = name));
    };
    let content = winc_core::diff::pick_hunks(&head, &disk, |key, old| {
        !skipped.iter().any(|s| s.matches(key, &old))
    });
    Ok(Partial {
        path: path.to_path_buf(),
        content: content.into_bytes(),
    })
}

/// 只暂存了一部分的文件 `path` 要提交成的内容：暂存区里的版本（按检出到工作区的形式）。
fn staged_content(git: &Git, root: &Path, path: &Path) -> Result<Partial, String> {
    let cancel = winc_editor::format::Cancel::default();
    let content = git.content_at(root, "", path, &cancel)?.unwrap_or_default();
    Ok(Partial {
        path: path.to_path_buf(),
        content,
    })
}

#[cfg(test)]
mod tests;
