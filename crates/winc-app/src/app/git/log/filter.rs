//! 筛选：按文字、分支、作者，以及一个文件的历史。

use winc_core::Lines;

use super::*;

use crate::app::scroll::set_scroll_line;

impl App {
    /// 当前日志的分支筛选条件。
    pub(crate) fn log_branch_filter(&self) -> Option<&str> {
        self.editor.active()?.log.as_ref()?.query.branch.as_deref()
    }

    /// 打开文件 `path` 的历史：一个只看改过这个文件的提交的日志标签（跟着改名往前找）；已经
    /// 开着就切过去。
    pub(in crate::app) fn git_file_history(&mut self, path: &Path) {
        self.open_history(path, None);
    }

    /// 当前文件中选中的几行（没有选区时是光标所在的一行）的历史（命令「Git：显示所选代码的
    /// 历史」）：只看改过这几行的提交。
    ///
    /// `git log -L` 按 `HEAD` 中的行号查找：未提交的改动先按与 `HEAD` 的差异换算，新加的行
    /// 换成它所在位置在 `HEAD` 中的那一行。
    pub(in crate::app) fn git_selection_history(&mut self) {
        self.fresh_line_changes();
        let Some(doc) = self.editor.active() else {
            return;
        };
        let Some(path) = doc.path.clone() else {
            self.flash(t!("git-no-file-in-tab"));
            return;
        };
        let text = doc.text();
        let (from, to) = (doc.selection.from(), doc.selection.to());
        let first = text.line_of(from);
        // 选区结束在下一行行首时，那一行没有被选中。
        let last = match text.line_of(to) {
            line if line > first && text.line_start(line) == to => line - 1,
            line => line,
        };
        let hunks = self.git.changes.hunks(doc.id);
        let range = (head_line(hunks, first) + 1, head_line(hunks, last) + 1);
        self.open_history(&path, Some(range));
    }

    /// 打开文件 `path` 的历史标签；`lines` 是只看的行范围（见 [`LogQuery::lines`]）。
    fn open_history(&mut self, path: &Path, lines: Option<(usize, usize)>) {
        let Some(root) = self.git.root.clone() else {
            self.flash(t!("git-not-a-repository"));
            return;
        };
        // 路径可能经过符号链接（macOS 的 `/var` 就是 `/private/var`）。
        let path = match winc_config::files::canonicalize(path) {
            Ok(real) if real.starts_with(&root) => real,
            _ if path.starts_with(&root) => path.to_path_buf(),
            _ => {
                self.flash(t!("git-file-outside-repository"));
                return;
            }
        };
        let title = match lines {
            Some((start, end)) => {
                let name = winc_editor::display_name(&path);
                let (start, end) = (start.to_string(), end.to_string());
                t!(
                    "git-lines-history-title",
                    name = name,
                    start = start,
                    end = end
                )
            }
            None => history_title(&path),
        };
        let (index, new) = self.editor.show_log(&title);
        let id = self.editor.documents[index].id;
        if new {
            if let Some(view) = self.editor.documents[index].log.as_deref_mut() {
                view.query.path = Some(path);
                view.query.lines = lines;
            }
            self.load_log(id);
        }
        self.focus = Focus::Editor;
        self.relayout();
        self.request_redraw();
    }

    /// 当前编辑的文件的历史（命令「Git：显示当前文件的历史」）。
    pub(in crate::app) fn git_current_file_history(&mut self) {
        match self.editor.active().and_then(|d| d.path.clone()) {
            Some(path) => self.git_file_history(&path),
            None => self.flash(t!("git-no-file-in-tab")),
        }
    }

    /// 筛选输入框里的文字变了：当前日志按新的文字从头再取。
    pub(in crate::app) fn log_filter_changed(&mut self) {
        let text = self.git.log.filter.text().to_owned();
        // 改了筛选文字即离开转到的提交，不再固定它。
        self.change_log_query(|q| {
            q.text = text;
            q.pin = None;
        });
    }

    /// 改当前日志的筛选条件，从头再取。
    pub(in crate::app) fn change_log_query(&mut self, change: impl FnOnce(&mut LogQuery)) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let id = doc.id;
        let Some(view) = doc.log.as_deref_mut() else {
            return;
        };
        let before = view.query.clone();
        change(&mut view.query);
        if view.query != before {
            // 选着的提交留着：新的结果里还有它就继续选它，详情一栏不在每次输入时先清空。
            view.reset();
            if let Some(doc) = self.editor.active_mut() {
                set_scroll_line(doc, 0.0);
            }
            self.load_log(id);
        }
        self.request_redraw();
    }

    /// 当前日志标签的筛选输入框里该有的文字（切换标签时换过来）。
    pub(crate) fn sync_log_filter(&mut self) {
        if self.focus == Focus::LogFilter {
            return;
        }
        let text = self
            .editor
            .active()
            .and_then(|d| d.log.as_deref())
            .map(|v| v.query.text.clone());
        if let Some(text) = text
            && self.git.log.filter.text() != text
        {
            self.git.log.filter.set_text(&text);
        }
    }

    /// 日志筛选输入框里的按键：`Esc` 有文字时清空、没有时回到列表，回车、`↓` 回到列表，
    /// 其余交给输入框。
    pub(in crate::app) fn on_key_log_filter(
        &mut self,
        event: &winit::event::KeyEvent,
        base: &str,
        cmd: bool,
    ) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) if !self.git.log.filter.text().is_empty() => {
                self.edit_field(|f| {
                    f.set_text("");
                    true
                });
            }
            Key::Named(NamedKey::Escape | NamedKey::Enter | NamedKey::ArrowDown) => {
                self.focus = Focus::Editor;
                self.request_redraw();
            }
            _ => self.on_key_field(event, base, cmd),
        }
    }

    /// 作者菜单里列的作者：已经取回来的提交里的作者，按提交数从多到少，最多 30 个
    /// （经验值：再长菜单就超出窗口高度）。
    pub(super) fn log_authors(&self) -> Vec<String> {
        let Some(view) = self.editor.active().and_then(|d| d.log.as_deref()) else {
            return Vec::new();
        };
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for commit in &view.commits {
            *counts.entry(commit.author.as_str()).or_default() += 1;
        }
        let mut authors: Vec<(&str, usize)> = counts.into_iter().collect();
        authors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        // 当前按作者筛选时，列表里只剩该作者：也列出来，以便打勾。
        let mut list: Vec<String> = authors
            .into_iter()
            .take(30)
            .map(|(a, _)| a.to_owned())
            .collect();
        if let Some(author) = &view.query.author
            && !list.contains(author)
        {
            list.insert(0, author.clone());
        }
        list
    }
}

/// 把当前文本的第 `line` 行（从 0 起）换成 `HEAD` 中对应的行：`hunks` 是从 `HEAD` 到当前文本的
/// 改动。改动之后的行按前面增减的行数平移；落在改动里的行换成这段改动在 `HEAD` 中的开头，
/// 那段在 `HEAD` 中没有内容（新加的行）时是它前面的一行。
pub(crate) fn head_line(hunks: &[winc_core::diff::LineHunk], line: usize) -> usize {
    let mut shift: isize = 0;
    for hunk in hunks {
        if line < hunk.new.start {
            break;
        }
        if line < hunk.new.end {
            return match hunk.old.is_empty() {
                true => hunk.old.start.saturating_sub(1),
                false => hunk.old.start,
            };
        }
        shift += hunk.old.len() as isize - hunk.new.len() as isize;
    }
    line.saturating_add_signed(shift)
}
