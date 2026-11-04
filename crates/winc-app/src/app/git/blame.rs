//! 逐行追溯（`git blame`）：行号栏左边多一栏，写每一行最后是谁、什么时候改的。
//!
//! 按文档开关（命令「Git：逐行追溯」、编辑区右键菜单）。打开以后在后台运行
//! `git blame --porcelain --contents -`，把编辑器里的内容（含未保存的修改）交给 git，行号与
//! 编辑器一致，改过还没提交的行算「未提交」；打字时不每次都运行 git，停手 [`BLAME_IDLE`] 或
//! 保存后再在后台重算。绘制在 `view/editor.rs` 的行号栏里。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use ropey::Rope;

use winc_core::Lines;
use winc_editor::DocumentId;
use winc_editor::format::Cancel;
use winc_editor::git::{Blame, BlameCommit};
use winc_i18n::t;
use winc_render::Rect;

use super::log::{display_at, local_text, short};
use super::unusable;
use crate::app::{App, UserEvent};

/// 逐行追溯那一栏的宽度（逻辑像素）。
///
/// 176 是经验值：容得下 7 位提交号、作者名和日期的常见长度。
pub(crate) const BLAME_W: f32 = 176.0;

/// 停止输入多久以后重新运行 `git blame`（时长，经验值 1 秒）。
///
/// 在这之前按改动就地挪动上一次的结果，连续输入时不反复运行 git。
const BLAME_IDLE: Duration = Duration::from_secs(1);

/// 一次逐行追溯的结果，以及该结果对应的文字。
struct Entry {
    /// 对应的文档版本。
    revision: u64,
    blame: Arc<Blame>,
    /// 算这份结果时的文字，下一次就地挪动时和新文字比。
    text: Rope,
}

/// 后台算好的一次逐行追溯。
struct Computed {
    doc: DocumentId,
    revision: u64,
    text: Rope,
    result: Result<Blame, String>,
}

/// 逐行追溯的状态。
pub(crate) struct BlameState {
    /// 开着逐行追溯的文档。
    on: HashSet<DocumentId>,
    /// 各文档最近一次的结果（后台算好的，或者按改动就地挪过的）。
    data: HashMap<DocumentId, Entry>,
    /// 正在后台算的文档和版本。
    computing: HashMap<DocumentId, u64>,
    /// 就地挪过、该在这个时刻重新运行 `git blame` 的文档。
    due: HashMap<DocumentId, Instant>,
    results: (Sender<Computed>, Receiver<Computed>),
}

impl Default for BlameState {
    fn default() -> Self {
        BlameState {
            on: HashSet::new(),
            data: HashMap::new(),
            computing: HashMap::new(),
            due: HashMap::new(),
            results: channel(),
        }
    }
}

impl BlameState {
    /// 文档 `id` 开着逐行追溯。
    pub(crate) fn is_on(&self, id: DocumentId) -> bool {
        self.on.contains(&id)
    }

    /// 文档 `id` 关闭了：释放它的逐行追溯。
    ///
    /// 正在算的结果交回来时文档已经不在 `on` 里，丢弃。
    pub(super) fn forget(&mut self, id: DocumentId) {
        self.on.remove(&id);
        self.data.remove(&id);
        self.computing.remove(&id);
        self.due.remove(&id);
    }

    /// 关闭所有文档的逐行追溯（模块关了、仓库换了）。
    pub(super) fn clear(&mut self) {
        self.on.clear();
        self.data.clear();
        self.computing.clear();
        self.due.clear();
    }
}

impl App {
    /// 当前文档开着逐行追溯时那一栏的宽度，否则为 0。
    ///
    /// 行号栏跟着变宽。
    pub(crate) fn blame_width(&self) -> f32 {
        match self.editor.active() {
            Some(doc) if self.blame.is_on(doc.id) && doc.shows_text() => BLAME_W,
            _ => 0.0,
        }
    }

    /// 开关当前文档的逐行追溯（命令「Git：逐行追溯」）。
    pub(in crate::app) fn toggle_blame(&mut self) {
        let Some(doc) = self.editor.active() else {
            return;
        };
        let id = doc.id;
        if self.blame.on.remove(&id) {
            self.blame.data.remove(&id);
            self.blame.due.remove(&id);
            self.relayout();
            self.request_redraw();
            return;
        }
        if self.blame_path(id).is_none() {
            self.flash(t!("git-blame-not-in-repository"));
            return;
        }
        self.blame.on.insert(id);
        self.relayout();
        self.update_blame(id);
        self.request_redraw();
    }

    /// 文档 `id` 在仓库里的文件路径；不是仓库里的文本文件时为 `None`。
    fn blame_path(&self, id: DocumentId) -> Option<(PathBuf, PathBuf)> {
        let root = self.git.root.clone()?;
        let doc = self.editor.document(id)?;
        if !doc.shows_text() || doc.is_large() {
            return None;
        }
        let path = doc.path.clone()?;
        let path = if path.starts_with(&root) {
            path
        } else {
            winc_config::files::canonicalize(path)
                .ok()
                .filter(|p| p.starts_with(&root))?
        };
        Some((root, path))
    }

    /// 绘制前调用：文档 `id` 开着逐行追溯、结果不是最新的就更新。
    ///
    /// 还没有结果、或者到了重算的时刻就在后台运行 `git blame`（同一时刻一个文档只算一次）；
    /// 文字刚变过就先按改动挪动上一次的结果，停止输入 [`BLAME_IDLE`] 以后再重算。
    pub(crate) fn update_blame(&mut self, id: DocumentId) {
        if !self.blame.is_on(id) {
            return;
        }
        let Some(doc) = self.editor.document(id) else {
            self.blame.on.remove(&id);
            return;
        };
        let revision = doc.revision;
        let Some(rope) = doc.text().rope() else {
            return;
        };
        if let Some(entry) = self.blame.data.get_mut(&id)
            && entry.revision != revision
        {
            entry.blame = Arc::new(shifted(&entry.blame, &entry.text, rope));
            entry.text = rope.clone();
            entry.revision = revision;
            self.blame.due.insert(id, Instant::now() + BLAME_IDLE);
        }
        let due = match self.blame.due.get(&id) {
            Some(at) => Instant::now() >= *at,
            None => !self.blame.data.contains_key(&id),
        };
        if !due || self.blame.computing.contains_key(&id) {
            return;
        }
        let rope = rope.clone();
        let Some((root, path)) = self.blame_path(id) else {
            return;
        };
        self.blame.due.remove(&id);
        self.blame.computing.insert(id, revision);
        let sender = self.blame.results.0.clone();
        self.spawn_git(UserEvent::GitBlame, move |tool| {
            let contents = rope.to_string();
            let result = match tool {
                Ok(git) => git.blame(&root, &path, contents.into_bytes(), &Cancel::default()),
                Err(e) => Err(unusable(&e)),
            };
            let _ = sender.send(Computed {
                doc: id,
                revision,
                text: rope,
                result,
            });
        });
    }

    /// 文档 `id` 刚保存：开着逐行追溯就马上重算，不等停手。
    pub(in crate::app) fn blame_saved(&mut self, id: DocumentId) {
        if self.blame.is_on(id) {
            self.blame.due.insert(id, Instant::now());
            self.update_blame(id);
        }
    }

    /// 事件循环每一轮调用：到了重算时刻的文档在后台重新运行；返回下一个重算时刻。
    pub(in crate::app) fn blame_tick(&mut self, now: Instant) -> Option<Instant> {
        let ready: Vec<_> = (self.blame.due.iter())
            .filter(|&(_, &at)| now >= at)
            .map(|(&id, _)| id)
            .collect();
        for id in ready {
            self.update_blame(id);
        }
        self.blame.due.values().min().copied()
    }

    /// 收下后台算好的逐行追溯。
    ///
    /// 失败时（文件还没被跟踪）说明原因并关闭。
    pub(in crate::app) fn poll_blame(&mut self) {
        while let Ok(done) = self.blame.results.1.try_recv() {
            self.blame.computing.remove(&done.doc);
            if !self.blame.is_on(done.doc) {
                continue;
            }
            match done.result {
                Ok(blame) => {
                    let entry = Entry {
                        revision: done.revision,
                        blame: Arc::new(blame),
                        text: done.text,
                    };
                    self.blame.data.insert(done.doc, entry);
                }
                Err(e) => {
                    self.blame.forget(done.doc);
                    self.relayout();
                    self.flash(t!("git-blame-failed", error = e));
                }
            }
        }
        self.request_redraw();
    }

    /// 文档 `id` 最近一次算好的逐行追溯（可能比文字旧一点）。
    pub(crate) fn blame_of(&self, id: DocumentId) -> Option<Arc<Blame>> {
        self.blame.data.get(&id).map(|e| e.blame.clone())
    }

    /// 鼠标在 `(x, y)` 时指着逐行追溯那一栏的第几行（从 0 起）。
    pub(crate) fn blame_at(&self, x: f32, y: f32) -> Option<usize> {
        let width = self.blame_width();
        let l = self.layout;
        if width == 0.0 || !(l.gutter.x..l.gutter.x + width).contains(&x) {
            return None;
        }
        if y < l.text.y || y > l.text.bottom() {
            return None;
        }
        let doc = self.editor.active()?;
        let row = ((y - l.text.y) / self.fonts.line_height + doc.scroll_line).floor() as usize;
        let line = doc.line_of_row(row);
        (line < doc.text().line_count()).then_some(line)
    }

    /// 第 `line` 行是哪次提交改的。
    pub(crate) fn blame_commit(&self, line: usize) -> Option<BlameCommit> {
        let doc = self.editor.active()?;
        let blame = self.blame_of(doc.id)?;
        let index = *blame.lines.get(line)?;
        blame.commits.get(index as usize).cloned()
    }

    /// 鼠标停在逐行追溯第 `line` 行上的提示：挂在哪里，和「a1b2c3d 说明 · 作者 · 时间」。
    pub(crate) fn blame_tip(&self, line: usize) -> Option<(Rect, String)> {
        let commit = self.blame_commit(line)?;
        let doc = self.editor.active()?;
        let l = self.layout;
        let lh = self.fonts.line_height;
        let y = l.text.y + (doc.row_of(line) as f32 - doc.scroll_line) * lh;
        let rect = Rect::new(l.gutter.x + 8.0, y, BLAME_W - 16.0, lh);
        let label = if commit.uncommitted {
            t!("git-blame-uncommitted").to_owned()
        } else {
            format!(
                "{} {} · {} · {}",
                short(&commit.oid),
                commit.summary,
                commit.author,
                local_text(commit.time, commit.offset)
            )
        };
        Some((rect, label))
    }

    /// 点了逐行追溯的第 `line` 行：在日志标签里选中那次提交。
    pub(in crate::app) fn click_blame(&mut self, line: usize) {
        let Some(commit) = self.blame_commit(line) else {
            return;
        };
        if commit.uncommitted {
            self.flash(t!("git-blame-line-uncommitted"));
            return;
        }
        self.git_show_log_at(&commit.oid);
    }
}

/// 逐行追溯一栏里写的时间：今天、昨天、几分钟前，或者日期（按作者所在时区）。
pub(crate) fn blame_time(commit: &BlameCommit, now: i64) -> String {
    display_at(commit.time + commit.offset, commit.time, now)
}

/// 文字从 `old` 改成 `new` 以后的逐行追溯：两头没动的行沿用 `blame` 里的提交，中间改过的行
/// 记成「未提交」。
///
/// 比的是整行，只看开头和结尾相同的部分，不运行 git；行数跟着改动增减（git 不追溯末尾换行
/// 后面那个空行，这里也不补）。
fn shifted(blame: &Blame, old: &Rope, new: &Rope) -> Blame {
    let head = old
        .lines()
        .zip(new.lines())
        .take_while(|(a, b)| a == b)
        .count();
    let (old_n, new_n) = (old.len_lines(), new.len_lines());
    let mut tail = 0;
    let (mut a, mut b) = (old.lines_at(old_n), new.lines_at(new_n));
    while tail < old_n.min(new_n) - head {
        match (a.prev(), b.prev()) {
            (Some(x), Some(y)) if x == y => tail += 1,
            _ => break,
        }
    }
    let mut commits = blame.commits.clone();
    let uncommitted = match commits.iter().position(|c| c.uncommitted) {
        Some(i) => i,
        None => {
            commits.push(BlameCommit {
                uncommitted: true,
                ..BlameCommit::default()
            });
            commits.len() - 1
        }
    } as u32;
    let row = |line: usize| blame.lines.get(line).copied().unwrap_or(uncommitted);
    let lines = (0..head)
        .map(row)
        .chain(std::iter::repeat_n(uncommitted, new_n - head - tail))
        .chain((old_n - tail..old_n).map(row))
        .take((blame.lines.len() + new_n).saturating_sub(old_n))
        .collect();
    Blame { commits, lines }
}

#[cfg(test)]
mod tests;
