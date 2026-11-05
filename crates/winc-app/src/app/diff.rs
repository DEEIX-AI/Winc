//! 并排差异标签的几何、命中区与交互。
//!
//! 负责标题栏按钮、折叠的未改动段、上一处与下一处改动之间的跳转，以及把左侧的一段内容
//! 替换到右侧。绘制由 `view` 子模块完成；可点击区域在绘制时记进 `diff_hits`，悬停与点击都查
//! 这张表。
//!
//! 右侧对应工作区文件时可直接编辑：右侧即编辑器里该文件的文档，光标、选区与撤销记录都属于该
//! 文档。在右侧点击放置光标后，按键与编辑命令转给该文档执行；文件未打开时先在后台打开一个
//! 标签，执行完重新比较。左侧（`HEAD`）只读。

mod pointer;

use std::path::Path;

use std::sync::mpsc::{Receiver, Sender, channel};
use winc_core::{Lines, Selection, movement};

use winc_editor::background;
use winc_editor::diff_view::{Compared, DiffView, LEFT, RIGHT, SideRow};
use winc_editor::{Document, DocumentId};
use winc_i18n::t;
use winc_render::Rect;
use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use super::git::changes::replace_lines;
use super::{App, Focus, UserEvent};
use crate::keymap::Action;

/// 差异标签顶上标题栏的高度（逻辑像素）：两侧的标题，右端是统计和按钮。
pub(crate) const DIFF_HEADER: f32 = 36.0;
/// 左侧行号左边放「»」的一列的宽度（逻辑像素）。
const DIFF_ARROWS: f32 = 22.0;
/// 右侧行号右边放勾选框的一列的宽度（逻辑像素）：按块提交时勾选每处改动。
const DIFF_CHECKS: f32 = 22.0;
/// 两侧行号之间连接改动的带子的宽度（逻辑像素）。
const DIFF_RIBBON: f32 = 36.0;
/// 行号左右各留的空白（逻辑像素）。
pub(crate) const DIFF_GUTTER_PAD: f32 = 10.0;
/// 文字和文字区边缘之间的空白（逻辑像素）。
pub(crate) const DIFF_TEXT_PAD: f32 = 10.0;

/// 「»」替换一段改动后，该段改动的颜色在原处淡出的时长（毫秒）。
///
/// 该值取自调参：长到能分辨哪几行被替换，又不至于让下一步操作等待。
const REPLACE_FADE: std::time::Duration = std::time::Duration::from_millis(420);

/// 一段改动的种类，决定它的颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HunkKind {
    /// 只在右侧有行。
    Added,
    /// 只在左侧有行。
    Deleted,
    /// 两侧都有行。
    Modified,
}

impl HunkKind {
    /// `hunk` 的种类。
    pub(crate) fn of(hunk: &winc_core::diff::LineHunk) -> HunkKind {
        match (hunk.old.is_empty(), hunk.new.is_empty()) {
            (true, _) => HunkKind::Added,
            (_, true) => HunkKind::Deleted,
            _ => HunkKind::Modified,
        }
    }
}

/// 正在淡出的替换颜色。
///
/// 记录两侧替换后对应的行号区间，以及改动原来的种类。
#[derive(Debug, Clone)]
pub(crate) struct DiffFade {
    /// 哪个差异标签。
    pub doc: DocumentId,
    /// 左侧的行。
    pub left: std::ops::Range<usize>,
    /// 右侧替换后的行，行数与 `left` 相同。
    pub right: std::ops::Range<usize>,
    /// 原来那段改动的种类。
    pub kind: HunkKind,
    /// 淡出的进度。
    pub clock: super::animation::Clock,
}

/// 右侧不超过该行数时当场重新比较，超过时放到后台（行）。
///
/// 该值取自调参：在 debug 构建中实测 2000 行约 4 ms；文档更大时若每个按键都同步比较，连续
/// 输入会明显滞后。
const REDIFF_IN_PLACE: usize = 3000;

/// 后台重新比较的一次结果。
pub(crate) struct Rediffed {
    /// 哪个差异标签。
    id: DocumentId,
    /// 比较时的左侧；换进去之前确认左侧没变。
    old: ropey::Rope,
    /// 比较时的右侧，即工作区文件那时的内容。
    new: ropey::Rope,
    /// 比较时右侧文档的版本号（`revision`），用于丢弃过时的结果。
    revision: u64,
    /// 比较时右侧文档是否有未保存的修改，决定右侧标题是否标出「未保存」。
    dirty: bool,
    /// 两侧的比较结果。
    compared: Compared,
}

/// 后台重新比较的状态。
pub(crate) struct RediffState {
    /// 正在后台比较的差异标签。
    running: std::collections::HashSet<DocumentId>,
    /// 后台比较结果的通道：发送端交给工作线程，接收端由界面线程轮询。
    results: (Sender<Rediffed>, Receiver<Rediffed>),
}

impl Default for RediffState {
    fn default() -> Self {
        RediffState {
            running: Default::default(),
            results: channel(),
        }
    }
}

/// 差异标签里可以点的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTarget {
    /// 上一处改动。
    Previous,
    /// 下一处改动。
    Next,
    /// 折起或展开所有没改的长段。
    Collapse,
    /// 折起来的一段，参数是第几段没改的内容。
    Fold(usize),
    /// 用左侧替换右侧的第几段改动，即把这一处改回 `HEAD` 的样子。
    Revert(usize),
    /// 提交时是否包含第几段改动。
    Include(usize),
}

/// 差异里点击、拖选的单位：单击按字，双击按词，三击（或点在行号上）按行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectUnit {
    Char,
    Word,
    Line,
}

impl SelectUnit {
    /// 连击 `count` 次对应的单位。
    fn of(count: u8) -> SelectUnit {
        match count {
            2 => SelectUnit::Word,
            3 => SelectUnit::Line,
            _ => SelectUnit::Char,
        }
    }

    /// `pos` 处按这个单位选中的范围（起点，终点）。
    fn around(self, text: &impl Lines, pos: usize) -> (usize, usize) {
        match self {
            SelectUnit::Char => (pos, pos),
            SelectUnit::Word => movement::word_at(text, pos),
            SelectUnit::Line => {
                let span = text.line_span(text.pos_to_line_col(pos).0);
                (span.start, span.end)
            }
        }
    }

    /// 从按下时选中的 `origin` 拖到 `pos`，返回（锚点，光标）：往前拖时锚点在 `origin` 的末尾，
    /// 按下时选中的词、行始终整个保留，鼠标在它内部抖动时也不会缩回。
    fn extend(self, text: &impl Lines, origin: (usize, usize), pos: usize) -> (usize, usize) {
        let (from, to) = self.around(text, pos);
        if from < origin.0 {
            (origin.1, from)
        } else {
            (origin.0, to.max(origin.1))
        }
    }
}

/// 右侧显示行上的一个点：对应右侧文档在哪个标签、第几行、第几个显示列。
struct DiffSpot {
    /// 右侧文档在 `documents` 中的下标。
    source: usize,
    /// 右侧文档中的行号（从 0 起）。
    line: usize,
    /// 显示列（从 0 起），制表符按当前 Tab 宽度展开。
    cell: usize,
    /// 点在最后一行下面。
    past_end: bool,
}

/// 差异的一侧在屏幕上的位置。
///
/// 左侧是文字在左、行号在右，右侧是行号在左、文字在右，两侧的
/// 行号都挨着中间的带子。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffPane {
    /// 整个这一侧（文字加行号，左侧还含「»」那一列）。
    pub rect: Rect,
    /// 行号栏。
    pub gutter: Rect,
    /// 文字区。
    pub text: Rect,
}

/// 差异标签的几何。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiffParts {
    /// 顶上的标题栏：两侧的标题各自对齐文字区，右端是第几处、增删行数、上一处、下一处、折叠
    /// 开关。
    pub header: Rect,
    /// 标题栏下面显示行的区域。
    pub body: Rect,
    /// 左侧（改之前）和右侧（改之后）。
    pub panes: [DiffPane; 2],
    /// 左侧行号左边放「»」的一列。
    pub arrows: Rect,
    /// 两侧行号之间连接改动的带子。
    pub ribbon: Rect,
    /// 右侧行号右边放勾选框的一列。
    pub checks: Rect,
}

impl App {
    /// 点了差异标签里的 `target`。
    pub(super) fn press_diff(&mut self, target: DiffTarget) {
        match target {
            DiffTarget::Previous => self.jump_difference(false),
            DiffTarget::Next => self.jump_difference(true),
            DiffTarget::Collapse => {
                let Some(doc) = self.editor.active_mut() else {
                    return;
                };
                if let Some(view) = doc.diff.as_deref_mut() {
                    // 右侧顶上那一行在切换以后还停在顶上。
                    let row = view.row_at(RIGHT, doc.scroll_line.max(0.0)) as usize;
                    let on = !view.collapses();
                    let row = view.set_collapse(on, row);
                    doc.scroll_line = view.pos_of(RIGHT, row as f32);
                }
                self.clamp_scroll();
            }
            DiffTarget::Fold(gap) => {
                if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) {
                    view.expand(gap);
                }
                self.clamp_scroll();
            }
            DiffTarget::Revert(hunk) => self.revert_diff_hunk(hunk),
            DiffTarget::Include(hunk) => self.toggle_diff_hunk(hunk),
        }
        self.request_redraw();
    }

    /// 差异标签对应提交视图里的哪个文件；不在提交视图里（没有改动、模块关着）时为 `None`。
    pub(crate) fn diff_entry(&self, view: &DiffView) -> Option<&winc_editor::git::Entry> {
        let path = view.repo_path.as_deref()?;
        self.git
            .status
            .as_ref()?
            .entries
            .iter()
            .find(|e| e.path == path && !e.is_dir)
    }

    /// 勾上或勾掉第 `hunk` 处改动：提交时只提交勾上的改动。
    fn toggle_diff_hunk(&mut self, hunk: usize) {
        let Some(view) = self.editor.active().and_then(|d| d.diff.as_deref()) else {
            return;
        };
        let all: Vec<(u64, std::ops::Range<usize>)> = view
            .keys
            .iter()
            .zip(&view.hunks)
            .map(|(&k, h)| (k, h.old.clone()))
            .collect();
        let Some((key, old)) = all.get(hunk).cloned() else {
            return;
        };
        let Some(entry) = self.diff_entry(view).cloned() else {
            return;
        };
        self.commit.toggle_hunk(&entry, key, &old, &all);
        self.request_redraw();
    }

    /// 取右侧对应的文档下标；未打开且 `open` 为 `true` 时就地打开，但不切换当前标签。
    fn diff_source(&mut self, path: &Path, open: bool) -> Option<usize> {
        let found = self
            .editor
            .documents
            .iter()
            .position(|d| d.path.as_deref() == Some(path));
        if let Some(index) = found {
            // 右侧要跟着文档编辑，尚未读取的先读进来。
            self.editor.load(index);
            return Some(index);
        }
        if !open {
            return None;
        }
        let active = self.editor.active;
        // 右侧要跟着文档编辑，要马上有内容：就地读。
        let index = self.editor.open(path);
        self.editor.active = active;
        self.editor.documents[index]
            .text_editable()
            .then_some(index)
    }

    /// 让差异标签 `index` 的右侧跟上编辑器里的文档。
    ///
    /// 文档发生变化（在差异里编辑、在文件的标签里编辑、撤销）时重新比较。
    pub(crate) fn sync_diff_source(&mut self, index: usize) {
        self.sync_diff(index, false);
    }

    /// 同步差异标签 `index` 的右侧，`now` 为 `true` 时不管文件多大都当场比较。
    ///
    /// 「»」替换完成后需要立即定位，因此要求结果当场可用。
    fn sync_diff(&mut self, index: usize, now: bool) {
        let Some(path) = self
            .editor
            .documents
            .get(index)
            .and_then(|d| d.diff.as_deref())
            .and_then(|v| v.new.path.clone())
        else {
            return;
        };
        let Some(source) = self.diff_source(&path, false) else {
            return;
        };
        let source = &self.editor.documents[source];
        let (revision, dirty) = (source.revision, source.is_dirty());
        let Some(rope) = source.text().rope().cloned() else {
            return;
        };
        let id = self.editor.documents[index].id;
        let Some(view) = self.editor.documents[index].diff.as_deref_mut() else {
            return;
        };
        if view.source == Some(revision) {
            return;
        }
        // 不超过 `REDIFF_IN_PLACE` 行的文档当场比较；更大的文档放到后台，同一标签同一时刻只
        // 比较一次，完成后若文档又变了再比较一次，连续输入不排队。
        let Some(proxy) = self
            .proxy
            .clone()
            .filter(|_| !now && rope.len_lines() > REDIFF_IN_PLACE)
        else {
            view.set_new_text(rope);
            view.source = Some(revision);
            view.new.title = worktree_title(dirty).to_owned();
            return;
        };
        if !self.rediff.running.insert(id) {
            return;
        }
        let old = view.old.text.clone();
        let sender = self.rediff.results.0.clone();
        background::spawn(move || {
            let compared = Compared::new(&old, &rope);
            let _ = sender.send(Rediffed {
                id,
                old,
                new: rope,
                revision,
                dirty,
                compared,
            });
            let _ = proxy.send_event(UserEvent::Rediffed);
        });
    }

    /// 收下后台重新比较的结果，换进差异标签。
    ///
    /// 文档在比较期间又变了时，下一帧再比较一次。
    pub(super) fn poll_rediffs(&mut self) {
        while let Ok(done) = self.rediff.results.1.try_recv() {
            self.rediff.running.remove(&done.id);
            let Some(view) = self
                .editor
                .document_mut(done.id)
                .and_then(|d| d.diff.as_deref_mut())
            else {
                continue;
            };
            // 比较期间左侧换了（`HEAD` 变了、标签换了内容），或者已经换上了更新的结果：作废。
            if view.source.is_some_and(|s| s >= done.revision) || view.old.text != done.old {
                continue;
            }
            view.apply(done.new, done.compared);
            view.source = Some(done.revision);
            view.new.title = worktree_title(done.dirty).to_owned();
            self.clamp_scroll();
            self.request_redraw();
        }
    }

    /// 用左侧内容替换右侧第 `hunk` 段改动。
    ///
    /// 文件已打开时修改编辑器里的文档，⌘Z 可撤销，保存仍然可用；未打开时直接写回磁盘，
    /// 不为这一处修改额外打开标签。
    pub(super) fn revert_diff_hunk(&mut self, hunk: usize) {
        let Some(index) = self.editor.active else {
            return;
        };
        self.sync_diff(index, true);
        let Some(view) = self.editor.documents[index].diff.as_deref() else {
            return;
        };
        let (Some(path), Some(h)) = (view.new.path.clone(), view.hunks.get(hunk).cloned()) else {
            return;
        };
        let inserted = view.old_lines(hunk);
        // 这段改动第一行离视图顶上多远：换完以后让同一行停在同一个地方。
        let offset = view.hunk_rows[RIGHT][hunk].start as f32
            - view.row_at(RIGHT, self.editor.documents[index].scroll_line);
        let doc_id = self.editor.documents[index].id;
        let fade = DiffFade {
            doc: doc_id,
            left: h.old.clone(),
            right: h.new.start..h.new.start + h.old.len(),
            kind: HunkKind::of(&h),
            clock: super::animation::Clock::new(REPLACE_FADE),
        };
        match self.diff_source(&path, false) {
            Some(source) => {
                if let Err(reason) =
                    replace_lines(&mut self.editor.documents[source], h.new, inserted)
                {
                    self.flash(reason);
                    return;
                }
                self.sync_diff(index, true);
            }
            None => {
                let written = Document::open(DocumentId(u64::MAX), path.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|mut doc| {
                        replace_lines(&mut doc, h.new, inserted).map_err(str::to_owned)?;
                        doc.save().map_err(|e| e.to_string())?;
                        Ok(doc.text().rope().cloned())
                    });
                match written {
                    Ok(rope) => {
                        if let Some((view, rope)) =
                            self.editor.documents[index].diff.as_deref_mut().zip(rope)
                        {
                            view.set_new_text(rope);
                        }
                    }
                    Err(e) => {
                        self.notify(t!("git-diff-revert-failed", error = e));
                        return;
                    }
                }
            }
        }
        // 换完以后这几行没有改动了，会被折进「n 行未改动」：只让这几行连同上下几行露着，其余
        // 仍然折起；视图不动，换上去的几行还在原处。
        if let Some(doc) = self.editor.documents.get_mut(index)
            && let Some(view) = doc.diff.as_deref_mut()
        {
            view.pin_new(fade.right.clone());
            let row = view.row_of_new(fade.right.start) as f32 - offset;
            doc.scroll_line = view.pos_of(RIGHT, row.max(0.0));
        }
        self.clamp_scroll();
        // 换掉的那段不是立即失去颜色，而是在原处淡出。系统设置了减少动态效果时不做。
        self.diff_fade = self
            .clock(REPLACE_FADE)
            .map(|clock| DiffFade { clock, ..fade });
        self.flash(t!("git-change-reverted"));
        self.request_redraw();
    }

    /// 在差异标签里执行「回滚光标处的改动」。
    ///
    /// 光标在右侧时回滚光标所在的那一处，否则回滚视口内可见的那一处。
    pub(super) fn revert_diff_hunk_here(&mut self) {
        let Some(index) = self.editor.active else {
            return;
        };
        self.sync_diff_source(index);
        let hunk = match self.diff_caret_line(index) {
            Some(_) => self.diff_caret_hunk(),
            None => {
                let doc = &self.editor.documents[index];
                doc.diff
                    .as_deref()
                    .and_then(|v| v.hunk_at_pos(doc.scroll_line.max(0.0) + 3.0))
            }
        };
        match hunk {
            Some(hunk) => self.revert_diff_hunk(hunk),
            None => self.flash(t!("git-diff-none-at-caret")),
        }
    }

    /// 光标在右侧、落在一处改动上时，那处改动是第几段。
    pub(super) fn diff_caret_hunk(&self) -> Option<usize> {
        let index = self.editor.active?;
        let line = self.diff_caret_line(index)?;
        let view = self.editor.documents[index].diff.as_deref()?;
        view.hunks
            .iter()
            .position(|h| h.new.contains(&line) || (h.new.is_empty() && h.new.start == line))
    }

    /// 光标在差异右侧时由右侧文档执行 `f`（按键、编辑命令），执行完重新比较并滚动到光标处。
    ///
    /// 光标不在右侧时返回 `None`，不做任何改动。
    fn in_diff_source<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        let index = self.editor.active?;
        let view = self.editor.documents[index].diff.as_deref()?;
        if !view.editing {
            return None;
        }
        let id = self.editor.documents[index].id;
        let path = view.new.path.clone()?;
        let source = self.diff_source(&path, true)?;
        self.editor.active = Some(source);
        let result = f(self);
        // `f` 可能关闭了标签：按标识找回差异标签。
        if let Some(index) = self.editor.index_of(id) {
            self.editor.active = Some(index);
            self.relayout();
            self.sync_diff_source(index);
            self.diff_caret_visible(index);
        }
        self.reset_blink();
        self.request_redraw();
        Some(result)
    }

    /// 让右侧光标可见：所在行被折叠时先展开，再滚动到视口内。
    /// 让当前的差异标签按设置和文字区宽度自动换行，视图停在原来的内容上；当前标签不是差异时返回
    /// `false`。
    ///
    /// 整篇断行的耗时与行数成正比：宽度逐帧变化时沿用已有的宽度，变化结束后再断行一次。
    pub(in crate::app) fn sync_diff_wrap(&mut self) -> bool {
        let wrap = self.config.word_wrap == Some(true);
        let tab_width = self.editor.active_tab_width();
        let resizing = self.text_width_changing();
        let (editor, cell) = (self.layout.editor, self.cell);
        let Some(doc) = self.editor.active_mut() else {
            return false;
        };
        let Some(view) = doc.diff.as_deref_mut() else {
            return false;
        };
        if resizing && wrap && view.is_wrapping() {
            return true;
        }
        let parts = diff_parts(editor, view, cell);
        let text_w = parts.panes[LEFT].text.w.min(parts.panes[RIGHT].text.w);
        let cols = ((text_w - 2.0 * DIFF_TEXT_PAD) / cell).floor().max(1.0) as usize;

        let row = view.row_at(RIGHT, doc.scroll_line.max(0.0));
        let (top, part) = match view.rows[RIGHT].get(row.floor() as usize) {
            Some(SideRow::Line { line, part, .. }) => (*line, *part),
            Some(SideRow::Fold { lines, .. }) => (lines.start, 0),
            None => (usize::MAX, 0),
        };
        if view.set_wrap(wrap.then_some((cols, tab_width))) {
            let part = part.min(view.breaks(RIGHT, top).rows().saturating_sub(1));
            let first = view.row_of_new(top) + part;
            doc.scroll_line = view.pos_of(RIGHT, first as f32 + row.fract());
            if view.is_wrapping() {
                doc.scroll_x = 0.0;
            }
            self.request_redraw();
        }
        true
    }

    fn diff_caret_visible(&mut self, index: usize) {
        let Some(line) = self.diff_caret_line(index) else {
            return;
        };
        let visible = self.visible_lines().saturating_sub(2).max(1);
        let cell = self.cell;
        let tab_width = self.editor.active_tab_width();
        let path = self.editor.documents[index]
            .diff
            .as_deref()
            .and_then(|v| v.new.path.clone());
        let (caret_col, caret_cell) = path
            .and_then(|p| self.diff_source(&p, false))
            .map(|s| {
                let doc = &self.editor.documents[s];
                let (line, col) = doc.text().pos_to_line_col(doc.selection.head);
                (col, doc.col_to_cell(line, col, tab_width))
            })
            .unwrap_or((0, 0));
        let width = {
            let Some(view) = self.editor.documents[index].diff.as_deref() else {
                return;
            };
            diff_parts(self.layout.editor, view, cell).panes[1].text.w - DIFF_TEXT_PAD
        };
        let doc = &mut self.editor.documents[index];
        let Some(view) = doc.diff.as_deref_mut() else {
            return;
        };
        view.reveal_new(line);
        // 按滚动位置比：左侧更长的改动里右侧停着不动，按右侧的行数比会差出一截。
        let at = view.pos_of(RIGHT, view.row_of_col(RIGHT, line, caret_col) as f32);
        if at < doc.scroll_line {
            doc.scroll_line = at;
        } else if at >= doc.scroll_line + visible as f32 {
            doc.scroll_line = at - visible as f32 + 1.0;
        }
        let x = if view.is_wrapping() {
            doc.scroll_x
        } else {
            caret_cell as f32 * cell
        };
        if x < doc.scroll_x {
            doc.scroll_x = (x - cell * 4.0).max(0.0);
        } else if x > doc.scroll_x + width - cell * 2.0 {
            doc.scroll_x = x - width + cell * 6.0;
        }
        self.clamp_scroll();
    }

    /// 在差异标签里执行查找 `f`，查找范围是右侧的工作区文件。
    ///
    /// 查找转给右侧文档执行，光标放到右侧，跳到的匹配即右侧的选区，差异随之滚动。
    ///
    /// 右侧不能编辑时返回 `None` 且不做任何改动：文件已删除，或比较的不是 Git 差异。
    pub(super) fn in_diff_search<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        let view = self.editor.active_mut()?.diff.as_deref_mut()?;
        view.new.path.as_ref()?;
        view.editing = true;
        view.left_selection = None;
        self.in_diff_source(f)
    }

    /// 当前标签是差异。
    pub(super) fn diff_active(&self) -> bool {
        self.editor.active().is_some_and(|d| d.diff.is_some())
    }

    /// 光标在差异右侧时的按键交给右侧的文档。
    ///
    /// 返回是否处理了。
    pub(super) fn diff_edit_key(&mut self, event: &KeyEvent, cmd: bool) -> bool {
        if event.logical_key == Key::Named(NamedKey::Escape) {
            let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut()) else {
                return false;
            };
            if view.left_selection.take().is_some() {
                self.request_redraw();
                return true;
            }
            if view.editing {
                view.editing = false;
                self.request_redraw();
                return true;
            }
            return false;
        }
        self.in_diff_source(|app| app.on_key_editor(event, cmd))
            .is_some()
    }

    /// 输入法提交的文字在光标在差异右侧时交给右侧的文档。
    ///
    /// 返回是否处理了。
    pub(super) fn diff_edit_text(&mut self, text: &str) -> bool {
        let text = text.to_owned();
        self.in_diff_source(|app| app.run(winc_editor::Command::Insert(text)))
            .is_some()
    }

    /// 光标在差异右侧时，把撤销、剪切、粘贴、保存、行操作等编辑命令交给右侧文档。
    ///
    /// 参与转发的命令由 [`Action::edits_in_diff`] 判定；返回 `None` 表示该命令不由右侧处理。
    pub(super) fn diff_edit_action(&mut self, action: Action) -> Option<bool> {
        use Action::*;
        // 左侧有选区时：复制选中的，全选选中整个左侧。
        if let Some(view) = self.editor.active_mut().and_then(|d| d.diff.as_deref_mut())
            && view.left_selection.is_some()
        {
            match action {
                Copy => {
                    if let Some(text) = view.left_selected() {
                        self.set_clipboard(text, false);
                    }
                    return Some(true);
                }
                SelectAll => {
                    view.left_selection = Some((0, view.old.text.len_chars()));
                    self.request_redraw();
                    return Some(true);
                }
                _ => {}
            }
        }
        if !action.edits_in_diff() {
            return None;
        }
        self.in_diff_source(|app| app.run_action(action))
    }

    /// 处理差异标签里的滚动按键。
    ///
    /// `↑` `↓` 滚动一行，翻页键滚动一屏，`⌘↑` `⌘↓`（其他系统为 `Ctrl+Home` `Ctrl+End`）
    /// 与 `Home` `End` 跳到开头和结尾，`←` `→` 横向滚动。其他按键不处理。
    pub(super) fn diff_key(&mut self, key: &Key, cmd: bool) {
        let page = self.visible_lines().saturating_sub(1).max(1) as f32;
        let cell = self.cell;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        match key {
            Key::Named(NamedKey::ArrowUp) if cmd => doc.scroll_line = 0.0,
            Key::Named(NamedKey::ArrowDown) if cmd => doc.scroll_line = f32::MAX,
            Key::Named(NamedKey::Home) => doc.scroll_line = 0.0,
            Key::Named(NamedKey::End) => doc.scroll_line = f32::MAX,
            Key::Named(NamedKey::ArrowUp) => doc.scroll_line -= 1.0,
            Key::Named(NamedKey::ArrowDown) => doc.scroll_line += 1.0,
            Key::Named(NamedKey::PageUp) => doc.scroll_line -= page,
            Key::Named(NamedKey::PageDown) => doc.scroll_line += page,
            Key::Named(NamedKey::ArrowLeft) => doc.scroll_x -= cell * 4.0,
            Key::Named(NamedKey::ArrowRight) => doc.scroll_x += cell * 4.0,
            _ => return,
        }
        doc.scroll_line = doc.scroll_line.floor();
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 滚动到下一处（`forward`）或上一处改动，使其停在距视口顶部三行的位置。
    ///
    /// 距离较远时平滑滚动。没有更多改动时，若差异由日志打开，则打开同一提交中的下一个文件；
    /// 否则在状态栏给出提示。
    pub(super) fn jump_difference(&mut self, forward: bool) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let Some(view) = doc.diff.as_deref() else {
            return;
        };
        // 判断光标当前所在段：刚跳转、尚未滚动时取跳转到的那一段（末尾几段无法滚到视口顶部，
        // 不能按滚动位置判断），否则按视口第四行判断。
        let here = doc.scroll_line.max(0.0).floor() + 3.0;
        let target = match (view.focus, view.current_hunk(doc.scroll_line)) {
            (Some((_, at)), Some(current)) if at == doc.scroll_line => {
                let next = if forward {
                    Some(current + 1)
                } else {
                    current.checked_sub(1)
                };
                next.filter(|&i| i < view.hunks.len())
            }
            _ => {
                let mut hunks = 0..view.hunks.len();
                if forward {
                    hunks.find(|&i| view.hunk_pos(i) > here)
                } else {
                    hunks.rev().find(|&i| view.hunk_pos(i) < here)
                }
            }
        };
        match target {
            Some(hunk) => {
                let before = doc.scroll_line;
                doc.scroll_line = (view.hunk_pos(hunk) - 3.0).max(0.0);
                self.clamp_scroll();
                self.glide_scroll(before);
                if let Some(doc) = self.editor.active_mut() {
                    let scroll = doc.scroll_line;
                    if let Some(view) = doc.diff.as_deref_mut() {
                        view.focus = Some((hunk, scroll));
                    }
                }
                self.request_redraw();
            }
            None => {
                let message = if view.is_identical() {
                    t!("git-diff-identical-sides")
                } else if forward {
                    t!("git-change-no-next")
                } else {
                    t!("git-change-no-previous")
                };
                // 差异从日志打开时，改为打开同一提交中的下一个（或上一个）文件。
                if !self.step_log_file(forward) {
                    self.flash(message);
                }
            }
        }
    }
}

/// 右侧（工作区）的标题。
pub(crate) fn worktree_title(dirty: bool) -> &'static str {
    if dirty {
        t!("git-diff-side-worktree-unsaved")
    } else {
        t!("git-diff-side-worktree")
    }
}

/// 在编辑区 `editor` 里计算差异 `view` 的几何布局。
///
/// 中间从左到右依次是「»」、左侧行号栏、带子、右侧行号栏、勾选框，两侧文字区平分剩余宽度；
/// 行号栏按两侧最大行号的位数定宽。
pub(crate) fn diff_parts(editor: Rect, view: &DiffView, cell: f32) -> DiffParts {
    let header = Rect::new(editor.x, editor.y, editor.w, DIFF_HEADER);
    let body = Rect::new(
        editor.x,
        header.bottom(),
        editor.w,
        (editor.h - DIFF_HEADER).max(0.0),
    );
    let lines = view.old.text.len_lines().max(view.new.text.len_lines());
    let digits = lines.to_string().len().max(2) as f32;
    let gutter_w = (digits * cell + 2.0 * DIFF_GUTTER_PAD).round();
    let middle = DIFF_ARROWS + 2.0 * gutter_w + DIFF_RIBBON + DIFF_CHECKS;
    let text_w = ((editor.w - middle) / 2.0).max(0.0).floor();

    let column = |x: f32, w: f32| Rect::new(x, body.y, w, body.h);
    let left_text = column(editor.x, text_w);
    let arrows = column(left_text.right(), DIFF_ARROWS);
    let left_gutter = column(arrows.right(), gutter_w);
    let ribbon = column(left_gutter.right(), DIFF_RIBBON);
    let right_gutter = column(ribbon.right(), gutter_w);
    let checks = column(right_gutter.right(), DIFF_CHECKS);
    let right_text = column(checks.right(), (editor.right() - checks.right()).max(0.0));
    DiffParts {
        header,
        body,
        panes: [
            DiffPane {
                rect: column(editor.x, left_gutter.right() - editor.x),
                gutter: left_gutter,
                text: left_text,
            },
            DiffPane {
                rect: column(right_gutter.x, editor.right() - right_gutter.x),
                gutter: right_gutter,
                text: right_text,
            },
        ],
        arrows,
        ribbon,
        checks,
    }
}
