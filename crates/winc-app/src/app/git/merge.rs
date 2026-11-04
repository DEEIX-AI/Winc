//! 在文件里解决合并冲突：每段冲突可分别采用当前、采用对方、两者都要，或打开并排对比。
//!
//! 有冲突标记的文件仍然在编辑区打开，不另开三栏窗口：每段冲突当前这一侧铺蓝、对方铺绿、祖先
//! 铺灰，`<<<<<<<` 那条上有四个按钮。选一边就是一次普通的修改，⌘Z 能撤销；右上角的卡片说还
//! 剩几处冲突，能跳上一处、下一处、一次全部采用一边，全解决后变成「标记为已解决」（`git add`）。
//! 绘制在 `view/git/merge.rs`，可以点的地方绘制时记进 `merge.hits`。

use std::sync::Arc;

use winc_core::Lines;
use winc_editor::DocumentId;
use winc_editor::diff_view::{DiffSide, DiffView};
use winc_editor::git::Change;
use winc_editor::git::merge::{self, Conflict, Pick};
use winc_i18n::t;
use winc_render::Rect;

use super::changes::replace_lines;
use crate::app::App;

/// 可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeTarget {
    /// 第几段冲突按某一边解决。
    Pick(usize, Pick),
    /// 并排对比第几段冲突的两侧。
    Compare(usize),
    /// 上一段冲突。
    Previous,
    /// 下一段冲突。
    Next,
    /// 所有冲突都按某一边解决。
    All(Pick),
    /// 冲突都解决了：`git add`，标记为已解决。
    MarkResolved,
}

/// 冲突解决的状态。
#[derive(Default)]
pub(crate) struct MergeState {
    /// 认出的冲突，按（文档，版本）缓存：文档不变时不必每帧重新扫一遍。
    cache: Option<(DocumentId, u64, Arc<Vec<Conflict>>)>,
    /// 可以点的地方，绘制时更新。
    pub hits: Vec<(Rect, MergeTarget)>,
    /// 右上角的卡片；没有时为 `None`。
    pub card: Option<Rect>,
}

impl App {
    /// 第 `index` 个标签里的冲突。
    ///
    /// Git 模块关着、不是文本、是大文件时没有。
    pub(crate) fn conflicts(&mut self, index: usize) -> Arc<Vec<Conflict>> {
        let enabled = self.git_enabled();
        let Some(doc) = self.editor.documents.get(index) else {
            return Arc::default();
        };
        let key = (doc.id, doc.revision);
        if let Some((id, revision, found)) = &self.merge.cache
            && (*id, *revision) == key
        {
            return found.clone();
        }
        let found = match doc.text().rope() {
            Some(rope) if enabled && doc.shows_text() => Arc::new(merge::find_conflicts(rope)),
            _ => Arc::default(),
        };
        self.merge.cache = Some((key.0, key.1, found.clone()));
        found
    }

    /// 当前文件在仓库状态里是不是还标着冲突（冲突标记都去掉了，但还没 `git add`）。
    pub(crate) fn active_unmerged(&self) -> bool {
        let Some(path) = self.editor.active().and_then(|d| d.path.as_deref()) else {
            return false;
        };
        self.git.status.as_ref().is_some_and(|s| {
            s.entries.iter().any(|e| {
                e.shown() == Change::Conflicted && winc_editor::canonical_path(&e.path) == path
            })
        })
    }

    /// 点了 `target`。
    pub(in crate::app) fn press_merge(&mut self, target: MergeTarget) {
        match target {
            MergeTarget::Pick(i, pick) => self.pick_conflict(i, pick),
            MergeTarget::Compare(i) => self.compare_conflict(i),
            MergeTarget::Previous => {
                self.jump_conflict(false);
            }
            MergeTarget::Next => {
                self.jump_conflict(true);
            }
            MergeTarget::All(pick) => self.pick_all_conflicts(pick),
            MergeTarget::MarkResolved => self.mark_resolved(),
        }
        self.request_redraw();
    }

    /// 第 `i` 段冲突按 `pick` 解决：整段换成选的那一边。
    fn pick_conflict(&mut self, i: usize, pick: Pick) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        let Some(conflict) = conflicts.get(i) else {
            return;
        };
        let doc = &mut self.editor.documents[index];
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let text = merge::resolution(rope, conflict, pick);
        if let Err(reason) = replace_lines(doc, conflict.lines(), text) {
            self.flash(reason);
            return;
        }
        let left = self.conflicts(index).len();
        self.flash(match (left, pick) {
            (0, _) => t!("git-merge-all-resolved").to_owned(),
            (n, Pick::Ours) => t!("git-merge-picked-current", count = n),
            (n, Pick::Theirs) => t!("git-merge-picked-incoming", count = n),
            (n, Pick::Both) => t!("git-merge-picked-both", count = n),
        });
    }

    /// 所有冲突都按 `pick` 解决。
    ///
    /// 从后往前换，前面的行号不受影响；每段是一次修改，可以一段段撤销。
    fn pick_all_conflicts(&mut self, pick: Pick) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        if conflicts.is_empty() {
            return;
        }
        let doc = &mut self.editor.documents[index];
        let Some(rope) = doc.text().rope().cloned() else {
            return;
        };
        for conflict in conflicts.iter().rev() {
            let text = merge::resolution(&rope, conflict, pick);
            if let Err(reason) = replace_lines(doc, conflict.lines(), text) {
                self.flash(reason);
                return;
            }
        }
        let count = conflicts.len();
        self.flash(match pick {
            Pick::Ours => t!("git-merge-picked-all-current", count = count),
            Pick::Theirs => t!("git-merge-picked-all-incoming", count = count),
            Pick::Both => t!("git-merge-picked-all-both", count = count),
        });
    }

    /// 并排对比第 `i` 段冲突的两侧：左边当前，右边对方。
    fn compare_conflict(&mut self, i: usize) {
        let Some(index) = self.editor.active else {
            return;
        };
        let conflicts = self.conflicts(index);
        let Some(conflict) = conflicts.get(i) else {
            return;
        };
        let doc = &self.editor.documents[index];
        let Some(rope) = doc.text().rope() else {
            return;
        };
        let lines = |range: std::ops::Range<usize>| -> String {
            range.flat_map(|l| rope.line(l).chunks()).collect()
        };
        let language = doc.highlighter.language();
        let name = doc.name().into_owned();
        let side = |label: &str, who: &str| {
            if label.is_empty() {
                who.to_owned()
            } else {
                format!("{who} · {label}")
            }
        };
        let view = DiffView::new(
            DiffSide::new(
                side(&conflict.ours_label, t!("git-merge-side-current")),
                &lines(conflict.ours()),
                None,
                language,
            ),
            DiffSide::new(
                side(&conflict.theirs_label, t!("git-merge-side-incoming")),
                &lines(conflict.theirs()),
                None,
                language,
            ),
        );
        let title = t!("git-merge-compare-title", name = name, index = i + 1);
        self.editor.show_diff(&title, view);
        self.relayout();
        self.clamp_scroll();
    }

    /// 跳到下一段（`forward`）或上一段冲突，让它停在离顶上三行的地方。
    ///
    /// 当前文件没有冲突时返回 `false`。
    pub(in crate::app) fn jump_conflict(&mut self, forward: bool) -> bool {
        let Some(index) = self.editor.active else {
            return false;
        };
        let conflicts = self.conflicts(index);
        if conflicts.is_empty() {
            return false;
        }
        let doc = &self.editor.documents[index];
        let caret = doc.text().pos_to_line_col(doc.selection.head).0;
        let target = if forward {
            conflicts.iter().find(|c| c.start > caret)
        } else {
            conflicts.iter().rev().find(|c| c.end < caret)
        };
        match target {
            Some(conflict) => {
                let pos = doc.text().line_start(conflict.start);
                self.select(pos, pos);
                let doc = &mut self.editor.documents[index];
                let row = doc.row_of(conflict.start);
                doc.scroll_line = row.saturating_sub(3) as f32;
                self.clamp_scroll();
            }
            None if forward => self.flash(t!("git-merge-no-next")),
            None => self.flash(t!("git-merge-no-previous")),
        }
        true
    }

    /// 冲突都解决了：先保存，再 `git add` 该文件，标记为已解决。
    fn mark_resolved(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        if !self.conflicts(index).is_empty() {
            self.flash(t!("git-merge-unresolved-left"));
            return;
        }
        let doc = &self.editor.documents[index];
        let (id, dirty) = (doc.id, doc.is_dirty());
        let Some(path) = doc.path.clone() else {
            return;
        };
        if dirty && !self.write_document(id, None, true) {
            return;
        }
        let name = winc_editor::display_name(&path).into_owned();
        self.git_operate(
            t!("git-op-mark-resolved", name = name.as_str()),
            move |git, root| {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                git.operate(root, &["add", "--", &relative.to_string_lossy()])
            },
        );
    }
}

/// 冲突里 `line` 那一行属于哪一部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Part {
    /// 标记行。
    Marker,
    /// 当前这一侧。
    Ours,
    /// 共同祖先。
    Base,
    /// 对方。
    Theirs,
}

/// `line` 在 `conflict` 里属于哪一部分；不在这段冲突里时为 `None`。
pub(crate) fn part_of(conflict: &Conflict, line: usize) -> Option<Part> {
    if !conflict.lines().contains(&line) {
        None
    } else if conflict.is_marker(line) {
        Some(Part::Marker)
    } else if conflict.ours().contains(&line) {
        Some(Part::Ours)
    } else if conflict.theirs().contains(&line) {
        Some(Part::Theirs)
    } else {
        Some(Part::Base)
    }
}
