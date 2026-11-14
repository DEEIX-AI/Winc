//! 日志左侧的分支面板：本地分支一组，远程分支每个远程一组。点一个分支只看它的提交，再点一次
//! 看全部；双击签出，右键是分支的各项操作（与分支选择器相同）。

use super::*;

/// 分支面板里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BranchRow {
    /// 最上面的「全部分支」：不按分支筛选。
    All,
    /// 一组的标题：`None` 是本地分支，否则是远程仓库的名字。点击收起、展开。
    Heading(Option<String>),
    /// 一个分支。
    Branch {
        /// 分支的名字，远程分支带远程名（`origin/main`）。
        name: String,
        /// 列表里写的名字：远程分支写在它的远程一组里，去掉远程名。
        label: String,
        /// 是否当前分支。
        current: bool,
    },
}

/// 分支面板一组里的一个分支：不是当前分支（排序时当前分支在前）、列表里写的名字、全名。
type Listed = (bool, String, String);

/// 选中一个提交以后过多久才去查所在分支。
///
/// 长于按住方向键时连续移动的间隔（系统默认的重复间隔不到 100 ms），连续移动时不查，停下
/// 以后很快就查。
pub(super) const BRANCHES_DELAY: Duration = Duration::from_millis(150);
/// 分支面板一行的高度（逻辑像素），与详情里文件行相同。
pub(crate) const BRANCH_ROW: f32 = LOG_FILE_ROW;

impl App {
    /// 在后台查找包含日志标签 `id` 选中的提交的分支（已经查到或正在查时不再查）；只查选中的，
    /// 不随详情预取：分支多、历史长时较慢，键盘连续移动时只查停下的那个。
    pub(super) fn fetch_log_branches(&mut self, id: DocumentId) {
        let Some(root) = self.git.root.clone() else {
            return;
        };
        let view = self.editor.document(id).and_then(|d| d.log.as_deref());
        let Some(details) = view.and_then(|v| v.details.as_ref()) else {
            return;
        };
        let oid = details.oid.clone();
        if details.branches.is_some() || !self.git.log.branches_fetching.insert(oid.clone()) {
            return;
        }
        let sender = self.git.log.results.0.clone();
        self.spawn_git(UserEvent::GitLog, move |tool| {
            let cancel = Cancel::default();
            let result = match tool {
                Ok(git) => git.containing_branches(&root, &oid, &cancel),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Loaded::Branches {
                doc: id,
                oid,
                result,
            });
        });
    }

    /// 选中了日志标签 `id` 里的一个提交：停下 [`BRANCHES_DELAY`] 以后再查包含它的分支（见
    /// [`App::log_tick`]）。上次没查出来的这次重新查。
    pub(super) fn schedule_log_branches(&mut self, id: DocumentId) {
        let view = self.editor.document(id).and_then(|d| d.log.as_deref());
        if let Some(oid) = view.and_then(|v| v.details.as_ref()).map(|d| &d.oid) {
            self.git.log.branches_failed.remove(oid);
        }
        self.git.log.branches_due = Some((id, Instant::now() + BRANCHES_DELAY));
    }

    /// 分支面板里的各行：「全部分支」，本地分支一组（当前分支在前），然后每个远程仓库一组；收起
    /// 的组只有标题。
    pub(crate) fn log_branch_rows(&self) -> Vec<BranchRow> {
        let mut rows = vec![BranchRow::All];
        // 远程分支按远程名分组：名字以「远程名/」开头的归那个远程，名字长的远程优先认。
        let mut remotes: Vec<&str> = self.git.remotes.iter().map(String::as_str).collect();
        remotes.sort_by_key(|r| std::cmp::Reverse(r.len()));
        let remote_of = |name: &str| {
            let owner = remotes.iter().find(|r| {
                name.strip_prefix(**r)
                    .is_some_and(|rest| rest.starts_with('/'))
            });
            owner.map_or_else(
                || name.split_once('/').map_or(name, |(r, _)| r).to_owned(),
                |r| (*r).to_owned(),
            )
        };
        let mut groups: Vec<(Option<String>, Vec<Listed>)> = Vec::new();
        for b in &self.git.branches {
            if b.remote && b.name.ends_with("/HEAD") {
                continue;
            }
            let (group, label) = match b.remote {
                false => (None, b.name.clone()),
                true => {
                    let remote = remote_of(&b.name);
                    let label = b.name[remote.len()..].trim_start_matches('/').to_owned();
                    (Some(remote), label)
                }
            };
            let entry = (!b.current, label, b.name.clone());
            match groups.iter_mut().find(|(g, _)| *g == group) {
                Some((_, list)) => list.push(entry),
                None => groups.push((group, vec![entry])),
            }
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        for (group, mut list) in groups {
            list.sort();
            let folded = self.git.log.branch_folds.contains(&group);
            rows.push(BranchRow::Heading(group));
            if folded {
                continue;
            }
            rows.extend(
                list.into_iter()
                    .map(|(other, label, name)| BranchRow::Branch {
                        name,
                        label,
                        current: !other,
                    }),
            );
        }
        rows
    }

    /// 双击了分支面板的第 `i` 行：是分支（不是当前分支）时签出，远程分支签出成同名的本地分支
    /// 并跟踪它，与分支选择器的「签出」相同。
    pub(in crate::app) fn checkout_log_branch_row(&mut self, i: usize) {
        let Some(BranchRow::Branch { name, .. }) = self.log_branch_rows().into_iter().nth(i) else {
            return;
        };
        let branch = self.git.branches.iter().find(|b| b.name == name).cloned();
        if let Some(branch) = branch.filter(|b| !b.current) {
            self.run_branch_op(crate::app::git::branch::BranchOp::Checkout, branch);
        }
    }

    /// 右键点在分支面板的一个分支上：返回它在分支列表（`GitState::branches`）里的下标，菜单
    /// 是分支选择器里同样的操作。
    pub(in crate::app) fn log_branch_context_target(&self, x: f32, y: f32) -> Option<usize> {
        if !self.log_active() {
            return None;
        }
        let LogTarget::BranchRow(i) = crate::layout::hit(&self.hits.log, x, y)? else {
            return None;
        };
        let BranchRow::Branch { name, .. } = self.log_branch_rows().into_iter().nth(i)? else {
            return None;
        };
        self.git.branches.iter().position(|b| b.name == name)
    }

    /// 分支面板露出的比例：关着为 0，开着为 1，打开、关闭途中在两者之间。
    pub(crate) fn log_branches_shown(&self) -> f32 {
        let open = f32::from(u8::from(self.state.log_branches.unwrap_or(true)));
        let motion = self.git.log.branches_motion;
        motion.map_or(open, |(from, clock)| clock.between(from, open))
    }

    /// 显示或隐藏分支面板，面板从左边滑进、滑出，与侧栏开合的节奏相同；所有窗口共用，下次
    /// 启动沿用。
    pub(super) fn toggle_log_branches(&mut self) {
        let from = self.log_branches_shown();
        let shown = !self.state.log_branches.unwrap_or(true);
        self.update_state(|state| state.log_branches = Some(shown));
        let clock = self.clock(crate::app::animation::SIDEBAR_ANIMATION);
        self.git.log.branches_motion = clock.map(|clock| (from, clock));
        self.relayout();
    }

    /// 点了分支面板的第 `i` 行：「全部分支」不再按分支筛选；分支只看它的提交，已经在看它时
    /// 改回全部；组的标题收起、展开这一组。
    pub(super) fn press_log_branch_row(&mut self, i: usize) {
        let Some(row) = self.log_branch_rows().into_iter().nth(i) else {
            return;
        };
        match row {
            BranchRow::All => self.change_log_query(|q| q.branch = None),
            BranchRow::Heading(group) => {
                let folds = &mut self.git.log.branch_folds;
                if !folds.remove(&group) {
                    folds.insert(group);
                }
            }
            BranchRow::Branch { name, .. } => {
                let same = self.log_branch_filter() == Some(name.as_str());
                self.change_log_query(|q| q.branch = (!same).then_some(name));
            }
        }
        self.request_redraw();
    }
}
