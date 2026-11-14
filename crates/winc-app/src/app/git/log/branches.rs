//! 日志左侧的分支面板：本地分支、远程分支各一组，点一个分支只看它的提交，再点一次看全部。

use super::*;

/// 分支面板里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BranchRow {
    /// 最上面的「全部分支」：不按分支筛选。
    All,
    /// 一组的标题：`false` 是本地分支，`true` 是远程分支。点击收起、展开。
    Heading(bool),
    /// 一个分支：名字（远程分支带远程名），是否当前分支。
    Branch(String, bool),
}

/// 分支面板一行的高度（逻辑像素），与详情里文件行相同。
pub(crate) const BRANCH_ROW: f32 = LOG_FILE_ROW;

impl App {
    /// 分支面板里的各行：「全部分支」，本地分支一组（当前分支在前），远程分支一组；收起的组只有
    /// 标题。没有远程分支时没有远程那一组。
    pub(crate) fn log_branch_rows(&self) -> Vec<BranchRow> {
        let mut rows = vec![BranchRow::All];
        for remote in [false, true] {
            let mut names: Vec<(bool, &str)> = (self.git.branches.iter())
                .filter(|b| b.remote == remote && !(remote && b.name.ends_with("/HEAD")))
                .map(|b| (!b.current, b.name.as_str()))
                .collect();
            if names.is_empty() {
                continue;
            }
            names.sort();
            rows.push(BranchRow::Heading(remote));
            if self.git.log.branch_folds[usize::from(remote)] {
                continue;
            }
            let branches = names
                .into_iter()
                .map(|(other, name)| BranchRow::Branch(name.to_owned(), !other));
            rows.extend(branches);
        }
        rows
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
            BranchRow::Heading(remote) => {
                let fold = &mut self.git.log.branch_folds[usize::from(remote)];
                *fold = !*fold;
            }
            BranchRow::Branch(name, _) => {
                let same = self.log_branch_filter() == Some(name.as_str());
                self.change_log_query(|q| q.branch = (!same).then_some(name));
            }
        }
        self.request_redraw();
    }
}
