//! 仓库配置的信任：读到状态后检查 `.git/config` 中会让 `git` 运行命令的配置，必要时询问用户。
//!
//! 信任过相同配置时正常使用，否则询问一次；用户回答之前不运行这些命令，修改仓库的操作被拒绝。
//! 两边都在界面线程上，`untrusted` 只读取后台已经查好的结果，不在界面线程上运行 `git`。

use super::*;

impl App {
    /// 检查仓库配置是否会让 `git` 运行命令（[`Git::untrusted`]），必要时询问用户是否信任。
    ///
    /// 信任过相同配置时正常使用，否则询问用户一次。用户回答之前，读取状态时不运行这些命令，
    /// 修改仓库的操作被拒绝。
    pub(super) fn check_git_trust(&mut self) {
        let (Some(root), Some(git)) = (self.git.root.clone(), self.git.tool.get()) else {
            return;
        };
        // 状态读取成功时后台已检查过配置，`untrusted` 只读取检查结果，不在界面线程上运行 `git`。
        if self.git.status.is_none() {
            return;
        }
        let Some(risks) = git.untrusted(&root) else {
            return;
        };
        if self.state.trusted_repo(&root) == Some(risks.fingerprint) {
            git.trust(&root, risks.fingerprint);
            return;
        }
        if self.git.asked_trust.contains(&risks.fingerprint) {
            return;
        }

        if self.window.handle.is_some() {
            use crate::app::AlertButton;

            let name = winc_editor::display_name(&root).into_owned();
            let message = t!("git-trust-confirm", name = name);
            let mut detail = t!("git-trust-confirm-detail").to_owned();
            detail.push('\n');
            // 详情中最多列出的配置项数（行）；取自调参：条目更多时详情的高度超出窗口。
            const SHOWN: usize = 8;
            for item in risks.items.iter().take(SHOWN) {
                detail.push_str("\n· ");
                detail.push_str(item);
            }
            if risks.items.len() > SHOWN {
                detail.push('\n');
                detail.push_str(&t!("git-trust-more", count = risks.items.len()));
            }
            let buttons = vec![
                AlertButton::cancel(t!("git-trust-deny")),
                AlertButton::new(t!("git-trust-allow")),
            ];
            let fingerprint = risks.fingerprint;
            let asked = self.present_alert(&message, &detail, buttons, None, {
                let root = root.clone();
                move |i, _| Some(UserEvent::GitTrust(root.clone(), fingerprint, i == 1))
            });
            // 窗口已有其他提示框时不显示，也不记录指纹，下次刷新时再询问。
            if asked {
                self.git.asked_trust.push(fingerprint);
            }
        }
    }

    /// 处理提示框中对是否信任仓库 `root` 配置的回答。
    pub(in crate::app) fn answer_git_trust(
        &mut self,
        root: &Path,
        fingerprint: u64,
        trusted: bool,
    ) {
        if !trusted {
            return;
        }
        self.update_state(|state| state.trust_repo(root, fingerprint));
        if let Some(git) = self.git.tool.get() {
            git.trust(root, fingerprint);
        }
        self.git_refresh();
    }
}
