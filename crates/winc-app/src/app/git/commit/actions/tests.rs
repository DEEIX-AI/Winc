//! 提交与回滚的测试：在指定仓库提交完成之后收下结果。

use super::*;

impl App {
    /// 测试用：在 `root` 里提交好了，收下结果。
    pub(in crate::app) fn committed_in(&mut self, root: &Path) {
        let outcome = Outcome::Committed(Ok("abc1234".into()), "msg".into(), root.into());
        let _ = self.commit.results.0.send(outcome);
        self.poll_commit();
    }
}

impl App {
    /// 模拟一个挂起的 AI 请求，返回发送网络结果的回调。
    pub(in crate::app) fn pending_ai_message(&mut self) -> impl FnOnce(&mut App, &str) + use<> {
        self.commit.ai_generation += 1;
        self.commit.ai_busy = true;
        let request = AiRequest {
            generation: self.commit.ai_generation,
            message: self.commit.message.text().to_owned(),
            checks: self.commit.checks,
            amend: self.commit.amend,
            head: self.commit.head.clone(),
            branch: self.git.status.as_ref().and_then(|s| s.head.clone()),
            entries: self
                .commit
                .checked_entries(self.git.status.as_ref().unwrap())
                .cloned()
                .collect(),
        };
        let root = self.git.root.clone().unwrap();
        move |app, message| {
            let _ = app.commit.results.0.send(Outcome::AiMessage(
                Ok(message.to_owned()),
                root,
                request,
            ));
            app.poll_commit();
        }
    }

    /// 取出 AI 真正使用的勾选内容，供部分行选择的回归测试验证。
    pub(in crate::app) fn ai_diff_for_checked(&self) -> String {
        let root = self.git.root.as_ref().unwrap();
        let git = self.git.tool.get().unwrap();
        let status = self.git.status.as_ref().unwrap();
        let paths: Vec<_> = self
            .commit
            .checked_entries(status)
            .flat_map(|e| std::iter::once(e.path.clone()).chain(e.from.clone()))
            .collect();
        let parts: Vec<_> = self
            .commit
            .checked_entries(status)
            .filter_map(|e| {
                self.commit
                    .skipped
                    .get(&e.path)
                    .map(|s| partial_content(&git, root, &e.path, s).unwrap())
            })
            .collect();
        crate::app::ai::prepare_commit_diff(&git, root, &paths, &parts, self.commit.amend).unwrap()
    }

    /// 模拟切换仓库时的清理，不丢弃仍在返回途中的异步消息。
    pub(in crate::app) fn reset_ai_context(&mut self) {
        self.reset_commit_view();
    }
}
