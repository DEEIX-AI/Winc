//! Git 模块的测试：准备差异失败时把原因交回界面线程。

use super::*;

impl App {
    /// 测试用：在 `root` 里给 `path` 准备差异失败（`Target::Open`）时，收下结果。
    pub(in crate::app) fn diff_failed_in(&mut self, root: &Path, path: &Path) {
        let _ = self.git.prepared.0.send(Prepared {
            kind: DiffKind::Head,
            root: root.into(),
            path: path.into(),
            oid: None,
            target: Target::Open,
            view: Err("读不出来".into()),
        });
        self.poll_git_content();
    }
}
