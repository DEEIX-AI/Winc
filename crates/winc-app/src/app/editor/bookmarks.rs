//! 书签的界面命令：切换、跳转到上一个或下一个、列出全部、清除。
//!
//! 书签按文件保存：关闭标签、退出程序后再次打开该文件，书签仍然存在；
//! 一个工作区目录的书签随该目录的现场保存。数据结构见 `winc_editor::bookmarks`。

use winc_i18n::t;

use super::*;
use crate::app::editor::definition::{Place, PlacesKind};

impl App {
    /// 切换光标所在行的书签（`⌥⌘K`）：没有则添加，已有则移除。
    ///
    /// 当前标签不显示文本，或是不对应文件的输出标签时不做任何事。
    pub(in crate::app) fn toggle_bookmark(&mut self) {
        self.apply_bookmark(None);
    }

    /// 添加（`on` 为真）或删除光标所在行的书签；对应菜单里的「添加书签」「删除书签」。
    pub(in crate::app) fn set_bookmark(&mut self, on: bool) {
        self.apply_bookmark(Some(on));
    }

    /// 让光标所在行的书签变成 `want`（`None` 表示切换）：已经是这样时不做任何事，否则切换并在
    /// 状态栏提示。
    ///
    /// 当前标签不显示文本，或是不对应文件的输出标签时不做任何事。
    fn apply_bookmark(&mut self, want: Option<bool>) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        if !doc.shows_text() || doc.title.is_some() {
            return;
        }
        if let Some(on) = want {
            let line = doc.text().line_of(doc.selection.head);
            if doc.bookmarks().lines().contains(&line) == on {
                return;
            }
        }
        let on = doc.toggle_bookmark();
        self.flash(if on {
            t!("editor-bookmark-added")
        } else {
            t!("editor-bookmark-removed")
        });
        self.request_redraw();
    }

    /// 跳转到光标之后（`forward` 为真）或之前的书签；越过最后一个或第一个书签时回绕到另一端。
    ///
    /// 当前文档没有书签时在状态栏提示。
    pub(in crate::app) fn cycle_bookmark(&mut self, forward: bool) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let line = doc.text().line_of(doc.selection.head);
        let target = if forward {
            doc.bookmarks().next(line)
        } else {
            doc.bookmarks().previous(line)
        };
        match target {
            Some(target) => self.jump_to(target, 0, 0),
            None => self.flash(t!("editor-bookmark-none")),
        }
    }

    /// 在快速打开的卡片中列出当前文档的书签（`⌥⌘L`）。
    ///
    /// 输入内容可筛选，回车跳转到选中的书签所在行；没有书签时在状态栏提示。
    pub(in crate::app) fn list_bookmarks(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let items: Vec<Place> = doc
            .bookmark_lines()
            .iter()
            .map(|&line| Place {
                path: doc.path.clone().unwrap_or_default(),
                relative: doc.name().into_owned(),
                line,
                col: None,
                preview: doc.bookmark_preview(line),
                marks: Vec::new(),
            })
            .collect();
        if items.is_empty() {
            self.flash(t!("editor-bookmark-none"));
            return;
        }

        self.show_places(PlacesKind::Bookmarks, String::new(), items);
        self.request_redraw();
    }

    /// 清除当前文档的全部书签（`⌘K ⌥⌘K`），并在状态栏提示清除的数量。
    pub(in crate::app) fn clear_bookmarks(&mut self) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let count = doc.bookmark_lines().len();
        if count == 0 {
            self.flash(t!("editor-bookmark-none"));
            return;
        }
        doc.clear_bookmarks();
        self.flash(t!("editor-bookmark-cleared", count = count));
        self.request_redraw();
    }
}
