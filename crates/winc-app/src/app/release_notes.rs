//! 更新日志：更新之后第一次启动时提示，按需从发布站点读取，在阅读标签中显示。

use winc_config::{State, compare_versions};
use winc_editor::Reading;
use winc_i18n::t;

use super::*;
use crate::release_notes::{self, Scope};

impl App {
    /// 记下当前运行的版本；这是更新之后的第一次启动时在右上角提示，提示上的按钮打开更新日志。
    ///
    /// 状态为空（第一次运行）、版本与上次运行时相同或更旧时不提示。
    pub(crate) fn note_version(&mut self) {
        let current = winc_version::VERSION;
        let previous = self.state.version.clone();
        if previous.as_deref() == Some(current) {
            return;
        }
        let first_run = self.state == State::default();
        self.update_state(|state| state.version = Some(current.to_owned()));
        let older = previous
            .as_deref()
            .is_some_and(|previous| compare_versions(current, previous).is_lt());
        if first_run || older {
            return;
        }
        self.notify_with(
            t!("workspace-release-notes-updated", version = current),
            NoticeAction::ReleaseNotes(previous),
        );
    }

    /// 打开「更新日志」标签并在后台读取：标签先显示正在读取，取回后换成 `scope` 范围内的各版
    /// （见 [`App::show_release_notes`]）。
    ///
    /// 没有事件循环（测试）时只打开标签，不请求网络。
    pub(in crate::app) fn open_release_notes(&mut self, scope: Scope) {
        self.editor
            .show_reading(t!("workspace-release-notes-title"), Reading::Loading);
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
        self.release_notes = scope.clone();
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
        let locale = winc_i18n::locale();
        winc_editor::background::spawn_blocking(move || {
            let notes = release_notes::load(locale, &scope);
            let _ = proxy.send_event(UserEvent::ReleaseNotes(notes));
        });
    }

    /// 重新读取更新日志，范围与上一次相同。
    pub(in crate::app) fn reload_release_notes(&mut self) {
        self.open_release_notes(self.release_notes.clone());
    }

    /// 把取回的更新日志 `notes` 换入「更新日志」标签，不切换标签；为 `None`（未能读取）时标签
    /// 中说明原因。标签在读取期间已经关闭时不再打开。
    pub(in crate::app) fn show_release_notes(&mut self, notes: Option<String>) {
        let reading = notes.map_or(Reading::Failed, |notes| Reading::Ready(notes.into()));
        let title = t!("workspace-release-notes-title");
        if self.editor.update_reading(title, reading) {
            self.relayout();
            self.request_redraw();
        }
    }

    /// 设置的「更新日志」：关闭设置，在阅读标签中打开全部版本。
    pub(in crate::app) fn settings_open_release_notes(&mut self) {
        self.close_settings();
        self.open_release_notes(Scope::All);
    }

    /// 在浏览器中打开完整的更新日志。
    pub(in crate::app) fn open_release_notes_page(&mut self) {
        let page = release_notes::page(winc_i18n::locale());
        if let Err(e) = crate::system::open_url(page) {
            self.flash(t!(
                "preview-markdown-link-open-failed",
                error = e.to_string()
            ));
        }
    }
}
