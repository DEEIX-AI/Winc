//! Git 的分支菜单：点状态栏的分支名弹出。
//!
//! 顶上是日志、获取、拉取、推送、新建分支、新建工作树和储藏，下面每个本地分支、远程分支各有
//! 子菜单：签出、在新工作树中签出、从这里新建分支、合并到当前分支、变基、删除。操作都在
//! 后台运行，同一时刻只做一件，运行期间状态栏的分支名后面写着在做什么；做完在状态栏说结论
//! （失败时右上角说原因）、刷新仓库状态，并重新读被改动过的文件。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use winc_editor::git::{Branch, Git, Operation, Stash};
use winc_i18n::t;

use super::unusable;
use super::worktree::RemoveCheck;
use crate::app::input::context_menu::{MenuCommand, MenuEntry};
use crate::app::{App, UserEvent};

/// 对一个分支做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchOp {
    /// 签出该分支；远程分支签出成同名的本地分支并跟踪它。
    Checkout,
    /// 在新工作树中签出（`App::new_worktree_for`），建好在新窗口打开。
    NewWorktree,
    /// 从该分支新建分支（输入名字）。
    NewFrom,
    /// 合并到当前分支。
    Merge,
    /// 把当前分支变基到该分支上。
    Rebase,
    /// 删除（只删已经合并的，`git branch -d`）。
    Delete,
    /// 和当前分支比较：打开日志，列出从当前分支到它改了的文件。
    Compare,
}

impl BranchOp {
    /// 全部操作，顺序就是子菜单里的顺序。
    pub const ALL: [BranchOp; 7] = [
        BranchOp::Checkout,
        BranchOp::NewWorktree,
        BranchOp::NewFrom,
        BranchOp::Compare,
        BranchOp::Merge,
        BranchOp::Rebase,
        BranchOp::Delete,
    ];

    /// 子菜单里的文字（不带分支名；菜单里实际显示的带分支名，见 `App::git_label`）。
    pub fn label(self) -> &'static str {
        match self {
            BranchOp::Checkout => t!("git-branch-checkout"),
            BranchOp::NewWorktree => t!("git-branch-checkout-new-worktree"),
            BranchOp::NewFrom => t!("git-branch-new-from"),
            BranchOp::Merge => t!("git-branch-merge-into-current"),
            BranchOp::Rebase => t!("git-branch-rebase-current-onto"),
            BranchOp::Delete => t!("git-branch-delete"),
            BranchOp::Compare => t!("git-branch-compare-with-current"),
        }
    }
}

/// 对一条储藏做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StashOp {
    /// 应用到工作区，储藏留着（`stash apply`）。
    Apply,
    /// 应用到工作区并删掉这条储藏（`stash pop`）。
    Pop,
    /// 删掉这条储藏（`stash drop`，先确认）。
    Drop,
}

impl StashOp {
    /// 全部操作，顺序就是子菜单里的顺序。
    pub const ALL: [StashOp; 3] = [StashOp::Apply, StashOp::Pop, StashOp::Drop];

    /// 子菜单里的文字。
    pub fn label(self) -> &'static str {
        match self {
            StashOp::Apply => t!("git-stash-apply"),
            StashOp::Pop => t!("git-stash-pop"),
            StashOp::Drop => t!("git-stash-drop"),
        }
    }
}

/// 一次操作成功之后要做的事。
pub(super) enum Then {
    /// 没有了：说一声做完了。
    Nothing,
    /// 工作树变了：重新取工作树列表。
    Refresh,
    /// 新建了工作树：重新取列表，打开它；`true` 是在新窗口打开。
    Open(PathBuf, bool),
    /// 检查完要移除的工作树：确认以后移除（[`App::confirm_remove_worktree`]），不说做完了。
    ConfirmRemove(RemoveCheck),
    /// 同步需要强制推送：确认以后推送（[`App::confirm_force_push`]），不说做完了；内容是上游将被
    /// 替换的提交数。
    ConfirmForcePush(usize),
}

/// 一次操作的结果。
struct Done {
    /// 第几个操作（[`BranchState::started`]）：换目录以后又起了新操作时，认得出这是之前的。
    id: u64,
    /// 在哪个仓库里做的。
    root: PathBuf,
    /// 做的是什么（「推送」「签出「dev」」），用在提示里。
    what: String,
    result: Result<(String, Then), String>,
}

/// 分支操作的状态。
pub(crate) struct BranchState {
    /// 正在做的事；状态栏的分支名后面写着它。
    pub running: Option<String>,
    /// 起过几个操作；正在做的是第几个。
    pub(super) started: u64,
    results: (Sender<Done>, Receiver<Done>),
}

impl Default for BranchState {
    fn default() -> Self {
        BranchState {
            running: None,
            started: 0,
            results: channel(),
        }
    }
}

/// 分支菜单的菜单项。
///
/// `branches` 是本地分支在前、远程分支在后的列表，当前分支只能新建分支；`held` 是被别的工作树
/// 签出的本地分支，它们的「签出」是切到那个工作树，不能删除；`stale` 是被目录已经不在的工作树
/// 占着的分支，git 仍然不允许签出、删除，要先清理。
pub(crate) fn branch_menu(
    branches: &[Branch],
    stashes: &[Stash],
    held: &[&str],
    stale: &[&str],
    operation: Option<Operation>,
    busy: bool,
) -> Vec<MenuEntry> {
    use MenuCommand::*;
    // 当前分支没有上游时拉取不了（推送可以：推上去并设为上游），灰掉，小字写着原因。
    let upstream = branches.iter().any(|b| b.current && b.upstream.is_some());
    let current = branches.iter().find(|b| b.current).map(|b| b.name.as_str());
    let mut entries = Vec::new();
    // 合并、变基停下来等着处理冲突：最上面是继续和中止。
    if operation.is_some() {
        entries.push(MenuEntry::Item(GitContinue, !busy));
        entries.push(MenuEntry::Item(GitAbort, !busy));
        entries.push(MenuEntry::Separator);
    }
    entries.extend([
        MenuEntry::Item(GitFetch, !busy),
        MenuEntry::Item(GitPull, !busy && upstream),
        MenuEntry::Item(GitPush, !busy),
        MenuEntry::Separator,
        MenuEntry::Item(GitNewBranch, !busy),
        MenuEntry::Item(NewWorktree, !busy),
        MenuEntry::Item(GitStash, !busy),
    ]);
    // 储藏：一条一个子菜单（应用、应用并删除、删除）。
    if !stashes.is_empty() {
        let items = stashes
            .iter()
            .enumerate()
            .map(|(i, stash)| {
                let mut ops: Vec<MenuEntry> = StashOp::ALL
                    .into_iter()
                    .map(|op| MenuEntry::Item(GitStashOp(op, i), !busy))
                    .collect();
                ops.insert(2, MenuEntry::Separator);
                MenuEntry::Submenu(stash_title(stash), ops)
            })
            .collect();
        entries.push(MenuEntry::Submenu(
            t!("git-stash-menu", count = stashes.len()),
            items,
        ));
    }
    // 本地分支：一个分支一个子菜单；当前分支在系统菜单的勾选列里打勾（和别的分支名对齐），
    // 子菜单里只有「从这里新建分支」。
    let ops = |i: usize, branch: &Branch| -> Vec<MenuEntry> {
        BranchOp::ALL
            .into_iter()
            .filter(|op| match op {
                // 当前分支不能签出、合并、变基、删除自己。
                _ if branch.current => *op == BranchOp::NewFrom,
                // 已经在其他工作树里签出着、又是当前分支（`origin/main` 在 `main` 上）：
                // 不能再签出该分支。
                BranchOp::NewWorktree => {
                    let local = local_name(branch);
                    !held.contains(&local) && !stale.contains(&local) && Some(local) != current
                }
                // 远程分支不在这里删除。
                BranchOp::Delete => !branch.remote,
                _ => true,
            })
            .map(|op| {
                // 被别的工作树签出的分支 git 不允许删除；签出就是切到那个工作树，不改仓库，忙的时候
                // 也能点。远程分支看签出后的本地分支。
                let elsewhere = held.contains(&local_name(branch));
                let gone = stale.contains(&local_name(branch));
                let enabled = match op {
                    BranchOp::Delete => !busy && !elsewhere && !gone,
                    BranchOp::Checkout if elsewhere => true,
                    BranchOp::Checkout if gone => false,
                    _ => !busy,
                };
                MenuEntry::Item(GitBranch(op, i), enabled)
            })
            .collect()
    };
    let local: Vec<_> = branches
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.remote)
        .collect();
    if !local.is_empty() {
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Item(GitHeading(false), false));
        for (i, branch) in local {
            let items = ops(i, branch);
            entries.push(if branch.current {
                MenuEntry::CheckedSubmenu(branch.name.clone(), items)
            } else {
                MenuEntry::Submenu(branch.name.clone(), items)
            });
        }
    }
    // 远程分支按远程收进子菜单（「origin ▸」），里面写去掉远程名的分支名：远程分支常常很多，
    // 平铺在第一层会把菜单拉得很长。
    let mut remotes: Vec<(&str, Vec<MenuEntry>)> = Vec::new();
    for (i, branch) in branches.iter().enumerate().filter(|(_, b)| b.remote) {
        let (remote, name) = branch.name.split_once('/').unwrap_or(("", &branch.name));
        let item = MenuEntry::Submenu(name.to_owned(), ops(i, branch));
        match remotes.iter_mut().find(|(r, _)| *r == remote) {
            Some((_, items)) => items.push(item),
            None => remotes.push((remote, vec![item])),
        }
    }
    if !remotes.is_empty() {
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Item(GitHeading(true), false));
        for (remote, items) in remotes {
            entries.push(MenuEntry::Submenu(remote.to_owned(), items));
        }
    }
    entries
}

/// 签出分支 `branch` 得到的本地分支名：远程分支去掉远程名（`origin/dev` → `dev`）。
pub(crate) fn local_name(branch: &Branch) -> &str {
    if branch.remote {
        branch
            .name
            .split_once('/')
            .map_or(branch.name.as_str(), |(_, l)| l)
    } else {
        &branch.name
    }
}

/// 储藏在菜单里的名字：git 记下的说明去掉「On main: 」这样的开头。
///
/// 超过 40 个字符的截短并加省略号（经验值：菜单一行放得下的长度）。
pub(crate) fn stash_title(stash: &Stash) -> String {
    let message = stash
        .message
        .split_once(": ")
        .map_or(stash.message.as_str(), |(_, rest)| rest);
    let mut title: String = message.chars().take(40).collect();
    if message.chars().count() > 40 {
        title.push('…');
    }
    title
}

impl App {
    pub(in crate::app) fn has_upstream(&self) -> bool {
        self.git
            .branches
            .iter()
            .any(|b| b.current && b.upstream.is_some())
    }

    /// 执行分支菜单里的 `command`。
    pub(in crate::app) fn run_branch_command(&mut self, command: MenuCommand) {
        use MenuCommand::*;
        match command {
            GitFetch => self.git_operate(t!("git-op-fetch").into(), |git, root| {
                git.operate(root, &["fetch", "--all", "--prune"])
            }),
            GitPull => self.git_operate(t!("git-op-pull").into(), |git, root| {
                git.operate(root, &["pull"])
            }),
            GitPush => {
                let upstream = self.has_upstream();
                self.git_operate(t!("git-op-push").into(), move |git, root| {
                    git.push(root, upstream)
                });
            }
            GitNewBranch => self.open_branch_dialog(None, None),
            NewWorktree => self.open_worktree_dialog(),
            GitStash => self.open_stash_prompt(),
            GitContinue | GitAbort => {
                let Some(operation) = self.git.status.as_ref().and_then(|s| s.operation) else {
                    return;
                };
                match command {
                    // 合并在提交视图里完成：写好（已经填上的）提交信息再提交。
                    GitContinue if operation == Operation::Merge => self.show_commit(),
                    GitContinue => self
                        .git_operate(continue_what(operation).into(), move |git, root| {
                            git.continue_operation(root, operation)
                        }),
                    _ => self.git_operate(abort_what(operation).into(), move |git, root| {
                        git.abort_operation(root, operation)
                    }),
                }
            }
            _ => {}
        }
    }

    /// 对储藏 `stash` 执行 `op`。
    ///
    /// 储藏按提交号指定（[`Git::stash`]），确认和执行期间其他储藏的增删不影响目标。
    pub(in crate::app) fn run_stash_op(&mut self, op: StashOp, stash: Stash) {
        let (name, oid) = (stash.name.clone(), stash.oid.clone());
        match op {
            StashOp::Apply => self.git_operate(
                t!("git-op-stash-apply", name = name.as_str()),
                move |git, root| git.stash(root, "apply", &oid),
            ),
            StashOp::Pop => self.git_operate(
                t!("git-op-stash-pop", name = name.as_str()),
                move |git, root| git.stash(root, "pop", &oid),
            ),
            StashOp::Drop => self.confirm_drop_stash(stash),
        }
    }

    /// 对分支 `branch` 执行分支菜单中的操作 `op`。
    pub(in crate::app) fn run_branch_op(&mut self, op: BranchOp, branch: Branch) {
        let name = branch.name.clone();
        match op {
            // 分支已经在其他工作树里签出：git 不允许重复签出，改为切到该工作树。
            BranchOp::Checkout if let Some(i) = self.git.worktree_holding(local_name(&branch)) => {
                self.open_worktree(i, true);
            }
            BranchOp::NewWorktree => self.new_worktree_for(&branch),
            BranchOp::Checkout if branch.remote => {
                // 已经有同名的本地分支就切过去，没有就建一个跟踪它的。
                let local = local_name(&branch);
                let exists = self
                    .git
                    .branches
                    .iter()
                    .any(|b| !b.remote && b.name == local);
                let local = local.to_owned();
                self.git_operate(
                    t!("git-op-checkout", name = local.as_str()),
                    move |git, root| {
                        if exists {
                            git.operate(root, &["switch", "--end-of-options", &local])
                        } else {
                            git.operate(root, &["switch", "--track", "--end-of-options", &name])
                        }
                    },
                );
            }
            BranchOp::Checkout => self.git_operate(
                t!("git-op-checkout", name = name.as_str()),
                move |git, root| git.operate(root, &["switch", "--end-of-options", &name]),
            ),
            BranchOp::NewFrom => {
                let from = (!branch.current).then_some(name);
                self.open_branch_dialog(from, None);
            }
            BranchOp::Merge => self.git_operate(
                t!("git-op-merge", name = name.as_str()),
                move |git, root| {
                    git.operate(root, &["merge", "--no-edit", "--end-of-options", &name])
                },
            ),
            BranchOp::Rebase => self.git_operate(
                t!("git-op-rebase-onto", name = name.as_str()),
                move |git, root| git.operate(root, &["rebase", "--end-of-options", &name]),
            ),
            BranchOp::Delete => self.git_operate(
                t!("git-op-delete-branch", name = name.as_str()),
                move |git, root| {
                    git.operate(root, &["branch", "--delete", "--end-of-options", &name])
                },
            ),
            BranchOp::Compare => self.git_compare_branch(&name),
        }
    }

    /// 储藏工作区和暂存区里的改动（含未跟踪的文件），说明是 `message`（空着时 git 自己写）。
    pub(in crate::app) fn git_stash(&mut self, message: &str) {
        let message = message.trim().to_owned();
        self.git_operate(t!("git-op-stash").into(), move |git, root| {
            let mut args = vec!["stash", "push", "--include-untracked"];
            if !message.is_empty() {
                args.extend(["--message", message.as_str()]);
            }
            git.operate(root, &args)
        });
    }

    /// 删除储藏前先确认（删掉就找不回来了）；没有窗口时（测试中）直接删。
    fn confirm_drop_stash(&mut self, stash: Stash) {
        let message = t!("git-stash-drop-confirm", name = stash_title(&stash));
        if self.window.is_some() {
            let detail = t!("git-stash-drop-confirm-detail");
            let button = t!("git-stash-drop-confirm-button");
            self.show_confirm(&message, detail, button, move |ok| {
                let op = super::log::LogConfirmed::DropStash(stash.clone());
                ok.then_some(UserEvent::GitLogConfirmed(op))
            });
            return;
        }
        // 没有窗口（测试中）时直接删除。
        self.run_log_confirmed(super::log::LogConfirmed::DropStash(stash));
    }

    /// 新建分支 `name`（从 `from` 开始，可以是分支或提交，`None` 是当前分支），`checkout` 为真时
    /// 切过去。
    pub(in crate::app) fn git_new_branch(
        &mut self,
        name: String,
        from: Option<String>,
        checkout: bool,
    ) {
        if name.is_empty() {
            return;
        }
        self.git_operate(
            t!("git-op-new-branch", name = name.as_str()),
            move |git, root| {
                // `--create` 把下一个参数当作它的值，`--end-of-options` 要放在名字之后。
                let mut args = if checkout {
                    vec!["switch", "--create", name.as_str(), "--end-of-options"]
                } else {
                    vec!["branch", "--end-of-options", name.as_str()]
                };
                if let Some(from) = from.as_deref() {
                    args.push(from);
                }
                git.operate(root, &args)
            },
        );
    }

    /// 在提交 `at` 上打一个轻量标签 `name`（`git tag name at`）；名字已经有了时 git 说原因。
    pub(in crate::app) fn git_new_tag(&mut self, name: &str, at: String) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.git_operate(
            t!("git-op-new-tag", name = name.as_str()),
            move |git, root| {
                git.operate(
                    root,
                    &["tag", "--end-of-options", name.as_str(), at.as_str()],
                )
            },
        );
    }

    /// 在后台做一件会改仓库的事 `what`，同一时刻只做一件。
    pub(super) fn git_operate(
        &mut self,
        what: String,
        work: impl FnOnce(&Git, &Path) -> Result<String, String> + Send + 'static,
    ) {
        self.git_operate_then(what, move |git, root| {
            work(git, root).map(|summary| (summary, Then::Nothing))
        });
    }

    /// 和 [`App::git_operate`] 一样，成功之后还要做 `work` 交回来的 [`Then`]。
    pub(super) fn git_operate_then(
        &mut self,
        what: String,
        work: impl FnOnce(&Git, &Path) -> Result<(String, Then), String> + Send + 'static,
    ) {
        if self.branch.running.is_some() {
            self.flash(t!("git-op-busy"));
            return;
        }
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let sender = self.branch.results.0.clone();
        self.branch.running = Some(what.clone());
        self.branch.started += 1;
        let id = self.branch.started;
        self.spawn_git(UserEvent::GitOperated, move |tool| {
            let result = match tool {
                Ok(git) => work(&git, &root),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Done {
                id,
                root,
                what,
                result,
            });
        });
        self.request_redraw();
    }

    /// 收下后台做完的分支操作。
    pub(in crate::app) fn poll_branch_ops(&mut self) {
        while let Ok(done) = self.branch.results.1.try_recv() {
            // 做的时候换了目录（换的时候 `running` 已经清掉，见 `git_folder_changed`）：只说一声在
            // 哪个仓库里做完了；新建的工作树仍然打开，检查完的不再移除。换回原来的目录、又起了新
            // 操作时根目录对得上但不是正在做的这个，同样按旧结果处理，不清掉新操作的 `running`。
            let stale = self.branch.running.is_none() || self.branch.started != done.id;
            if stale || self.git.root.as_deref() != Some(done.root.as_path()) {
                let name = winc_editor::display_name(&done.root);
                match done.result {
                    Ok((_, Then::ConfirmRemove(_) | Then::ConfirmForcePush(_))) => {}
                    Ok((_, then)) => {
                        self.flash(t!("git-op-done-in", name = name, what = done.what));
                        // 已经换了目录：当前窗口不再换过去，在新窗口打开。
                        if let Then::Open(path, _) = then {
                            self.open_in_new_window(&path);
                        }
                    }
                    Err(e) => {
                        let what = done.what;
                        self.notify(t!("git-op-failed-in", name = name, what = what, error = e));
                    }
                }
                continue;
            }
            self.branch.running = None;
            // 操作由表单对话框发起时：成功则关闭对话框，失败则在对话框中显示原因。
            let error = done.result.as_ref().err().map(String::as_str);
            if self.form_operation_done(done.id, error) && error.is_some() {
                continue;
            }
            match done.result {
                Ok((_, Then::ConfirmRemove(check))) => self.confirm_remove_worktree(check),
                Ok((_, Then::ConfirmForcePush(count))) => self.confirm_force_push(count),
                Ok((summary, then)) => {
                    if summary.is_empty() {
                        self.flash(t!("git-op-done", what = done.what));
                    } else {
                        let what = done.what;
                        self.flash(t!("git-op-done-summary", what = what, summary = summary));
                    }
                    self.worktree_done(then);
                }
                Err(e) => self.notify(t!("git-op-failed", what = done.what, error = e)),
            }
            // 切换分支、拉取、合并都会改工作区里的文件。
            self.check_disk();
        }
        self.request_redraw();
    }
}

/// 继续 `operation` 时提示中「做的是什么」的文字。
fn continue_what(operation: Operation) -> &'static str {
    match operation {
        Operation::Merge => t!("git-op-continue-merge"),
        Operation::Rebase => t!("git-op-continue-rebase"),
        Operation::CherryPick => t!("git-op-continue-cherry-pick"),
        Operation::Revert => t!("git-op-continue-revert"),
    }
}

/// 中止 `operation` 时提示中「做的是什么」的文字。
fn abort_what(operation: Operation) -> &'static str {
    match operation {
        Operation::Merge => t!("git-op-abort-merge"),
        Operation::Rebase => t!("git-op-abort-rebase"),
        Operation::CherryPick => t!("git-op-abort-cherry-pick"),
        Operation::Revert => t!("git-op-abort-revert"),
    }
}

#[cfg(test)]
mod tests;
