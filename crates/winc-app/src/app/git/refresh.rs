//! Git 视图可见时补查本地状态，恢复目录监视未报告的改动。

use std::time::Instant;

use super::STATUS_RECHECK;
use crate::app::App;

impl App {
    /// Git 视图可见且窗口有焦点时低频补查本地状态，兜底系统未报告的文件变化。
    ///
    /// 通知仍负责即时刷新；补查只读取状态，不读取分支、储藏或访问远程仓库。
    pub(in crate::app) fn git_status_tick(&mut self, now: Instant) -> Option<Instant> {
        let visible = self.commit_shown()
            || self
                .editor
                .active()
                .is_some_and(|d| d.log.is_some() || d.diff.is_some());
        let local = self
            .git
            .root
            .as_deref()
            .is_some_and(|root| !winc_backend::is_remote(root));
        if !self.git_enabled() || !local || !visible || !self.window.focused || self.unseen() {
            self.git.recheck_due = None;
            return None;
        }
        let due = *self.git.recheck_due.get_or_insert(now + STATUS_RECHECK);
        if now >= due {
            self.git.recheck_due = Some(now + STATUS_RECHECK);
            if self.git.running.is_none()
                && self.git.branch.running.is_none()
                && !self.git.commit.staging_active()
            {
                self.git_refresh_status();
            }
        }
        self.git.recheck_due
    }
}
