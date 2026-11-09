//! 从 Git 日志打开的差异所对应的只读文件树：列出该次提交或比较改动的文件，点击文件在同一个
//! 差异标签中切换。

use std::collections::BTreeMap;
use std::ffi::OsStr;

use super::*;
use crate::layout::SidebarView;
use crate::theme::TREE_ROW;

/// 文件树中的一行：目录，或文件在 [`LogDiffTree::files`] 中的下标。
pub(crate) enum LogDiffRow {
    /// 目录，路径相对仓库根目录。
    Dir(PathBuf),
    /// 文件在 [`LogDiffTree::files`] 中的下标。
    File(usize),
}

/// 差异标签的文件树。展开状态与工作区文件树无关。
pub(crate) struct LogDiffTree {
    /// 差异所属的提交或比较，以及其中改动的文件。
    pub(super) source: Arc<LogDiffSource>,
    /// 当前打开的文件在 `source.files` 中的下标。
    pub(super) selected: usize,
    /// 各文件在树中按显示顺序排列（目录全部展开时）的下标，用于逐个切换文件。
    pub(super) order: Vec<usize>,
    /// 当前显示的各行：（缩进层级，行）。
    pub rows: Vec<(usize, LogDiffRow)>,
    /// 已收起的目录，路径相对仓库根目录。
    pub collapsed: HashSet<PathBuf>,
    /// 列表的纵向滚动量（逻辑像素）。
    pub scroll: f32,
    /// 是否在提交视图中显示该树；为 `false` 时提交视图显示工作区的改动。
    pub(super) shown: bool,
}

impl LogDiffTree {
    /// 创建 `source` 的文件树，选中第 `selected` 个文件，目录全部展开。
    fn new(source: Arc<LogDiffSource>, selected: usize) -> Self {
        let mut order = Vec::new();
        append_rows(
            &mut order,
            &HashSet::new(),
            Path::new(""),
            0,
            relative_paths(&source),
        );
        let mut tree = LogDiffTree {
            order: (order.into_iter())
                .filter_map(|(_, row)| match row {
                    LogDiffRow::File(index) => Some(index),
                    LogDiffRow::Dir(_) => None,
                })
                .collect(),
            source,
            selected,
            rows: Vec::new(),
            collapsed: HashSet::new(),
            scroll: 0.0,
            shown: true,
        };
        tree.rebuild();
        tree
    }

    /// 返回标题中显示的版本：提交号，或比较的两个提交号。
    pub fn version(&self) -> &str {
        &self.source.version
    }

    /// 返回这次提交或比较改动的文件。
    pub fn files(&self) -> &[CommitFile] {
        &self.source.files
    }

    /// 返回当前打开的文件在 [`LogDiffTree::files`] 中的下标。
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 返回按树的显示顺序与当前文件相邻的文件：（下标，在显示顺序中的位置）；`forward` 为
    /// `true` 时取下一个，没有时返回 `None`。
    pub(super) fn neighbor(&self, forward: bool) -> Option<(usize, usize)> {
        let at = self.order.iter().position(|&i| i == self.selected)?;
        let next = if forward { at + 1 } else { at.checked_sub(1)? };
        self.order.get(next).map(|&i| (i, next))
    }

    /// 按收起的目录重新排出 `rows`。
    fn rebuild(&mut self) {
        self.rows.clear();
        let paths = relative_paths(&self.source);
        append_rows(&mut self.rows, &self.collapsed, Path::new(""), 0, paths);
    }
}

/// 返回 `source` 中各文件的（下标，相对仓库根目录的路径）。
fn relative_paths(source: &LogDiffSource) -> impl Iterator<Item = (usize, &Path)> {
    let root = &source.root;
    (source.files.iter().enumerate())
        .map(move |(i, file)| (i, file.path.strip_prefix(root).unwrap_or(&file.path)))
}

/// 把 `paths`（下标，相对 `parent` 的路径）排成树的各行追加到 `rows`，层级从 `depth` 起。
///
/// 目录在前、文件在后，同类按自然排序（[`winc_editor::natural_cmp`]），与工作区文件树一致；
/// `collapsed` 中的目录只占一行。
fn append_rows<'a>(
    rows: &mut Vec<(usize, LogDiffRow)>,
    collapsed: &HashSet<PathBuf>,
    parent: &Path,
    depth: usize,
    paths: impl IntoIterator<Item = (usize, &'a Path)>,
) {
    let mut dirs: BTreeMap<&OsStr, Vec<_>> = BTreeMap::new();
    let mut files = Vec::new();
    for (index, path) in paths {
        let mut parts = path.components();
        let first = parts.next();
        let rest = parts.as_path();
        if let Some(first) = first.filter(|_| !rest.as_os_str().is_empty()) {
            dirs.entry(first.as_os_str())
                .or_default()
                .push((index, rest));
        } else {
            files.push((path, index));
        }
    }
    let name = |s: &OsStr| s.to_string_lossy().into_owned();
    let mut dirs: Vec<_> = dirs.into_iter().collect();
    dirs.sort_by(|a, b| winc_editor::natural_cmp(&name(a.0), &name(b.0)));
    for (name, children) in dirs {
        let dir = parent.join(name);
        let open = !collapsed.contains(&dir);
        rows.push((depth, LogDiffRow::Dir(dir.clone())));
        if open {
            append_rows(rows, collapsed, &dir, depth + 1, children);
        }
    }
    files.sort_by(|a, b| winc_editor::natural_cmp(&name(a.0.as_os_str()), &name(b.0.as_os_str())));
    rows.extend(
        files
            .into_iter()
            .map(|(_, index)| (depth, LogDiffRow::File(index))),
    );
}

impl App {
    /// 返回提交视图中显示的历史差异文件树：当前标签是从日志打开的差异、侧栏显示提交视图，
    /// 且该树没有被切换为工作区改动时才有。
    pub(crate) fn log_diff_tree(&self) -> Option<&LogDiffTree> {
        let id = self.editor.active()?.id;
        (self.sidebar_view == SidebarView::Commit && self.steps_log_files(id))
            .then_some(self.log.tree.as_ref())
            .flatten()
            .filter(|tree| tree.shown)
    }

    /// 在提交视图中切换显示历史差异文件树与工作区改动：提交视图已经可见时切换，
    /// 否则显示文件树。当前标签不是从日志打开的差异时不做处理。
    pub(in crate::app) fn toggle_log_diff_tree(&mut self) {
        let visible = self.panel_visible && self.sidebar_view == SidebarView::Commit;
        let Some(id) = self.editor.active().map(|doc| doc.id) else {
            return;
        };
        if self.steps_log_files(id)
            && let Some(tree) = &mut self.log.tree
        {
            tree.shown = !visible || !tree.shown;
        }
    }

    /// 返回文件树列表的区域：侧栏中标题行以下的部分。
    pub(crate) fn log_diff_list(&self) -> Rect {
        let panel = self.panel_rect();
        let top = self.layout.sidebar_header.bottom();
        Rect::new(panel.x, top, panel.w, (panel.bottom() - top).max(0.0))
    }

    /// 返回窗口坐标 (`x`, `y`) 处文件树的行号（在 `rows` 中的下标）；不在某一行上时返回
    /// `None`。
    pub(crate) fn log_diff_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let tree = self.log_diff_tree()?;
        let list = self.log_diff_list();
        if !list.contains(x, y) {
            return None;
        }
        let row = ((y - list.y + tree.scroll) / TREE_ROW) as usize;
        (row < tree.rows.len()).then_some(row)
    }

    /// 把文件树滚动到 `scroll`（逻辑像素），限制在有效范围内。
    pub(in crate::app) fn scroll_log_diff(&mut self, scroll: f32) {
        let height = self.log_diff_list().h;
        if self.log_diff_tree().is_some()
            && let Some(tree) = &mut self.log.tree
        {
            tree.scroll = scroll.clamp(0.0, (tree.rows.len() as f32 * TREE_ROW - height).max(0.0));
        }
    }

    /// 处理点击文件树第 `index` 行：目录展开或收起，文件在同一个差异标签中打开。
    ///
    /// 点击当前已选中的文件时，作废尚未返回的其他文件的读取，使最后一次点击的文件保持显示。
    pub(in crate::app) fn click_log_diff_row(&mut self, index: usize) {
        if self.log_diff_tree().is_none() {
            return;
        }
        let Some(tree) = &mut self.log.tree else {
            return;
        };
        match tree.rows.get(index).map(|(_, row)| row) {
            Some(LogDiffRow::Dir(path)) => {
                if !tree.collapsed.remove(path) {
                    tree.collapsed.insert(path.clone());
                }
                tree.rebuild();
                let scroll = tree.scroll;
                self.scroll_log_diff(scroll);
            }
            Some(LogDiffRow::File(file)) if *file != tree.selected => {
                let file = *file;
                self.diff_land = None;
                self.focus = Focus::Editor;
                self.open_log_diff_file(file);
            }
            Some(LogDiffRow::File(_)) => {
                self.log.diff_request += 1;
                self.diff_land = None;
                self.focus = Focus::Editor;
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// 在差异标签中打开当前文件树中的第 `file` 个文件（在后台读取）。
    pub(super) fn open_log_diff_file(&mut self, file: usize) {
        if let Some(tree) = &self.log.tree {
            self.load_log_diff(Arc::clone(&tree.source), file);
        }
    }

    /// 记录差异标签显示的是 `source` 的第 `file` 个文件，并展开该文件所在的目录。
    ///
    /// `source` 与当前文件树相同时保留展开状态和滚动位置，否则换成新的文件树。
    pub(super) fn select_log_diff(&mut self, source: Arc<LogDiffSource>, file: usize) {
        let keep = self
            .log
            .tree
            .as_mut()
            .filter(|tree| Arc::ptr_eq(&tree.source, &source) || tree.source == source);
        let tree = if let Some(tree) = keep {
            tree.selected = file;
            tree.shown = true;
            tree
        } else {
            self.log.tree.insert(LogDiffTree::new(source, file))
        };
        let path = &tree.source.files[file].path;
        if let Ok(relative) = path.strip_prefix(&tree.source.root) {
            let before = tree.collapsed.len();
            tree.collapsed.retain(|dir| !relative.starts_with(dir));
            if tree.collapsed.len() != before {
                tree.rebuild();
            }
        }
    }

    /// 滚动文件树，使当前打开的文件可见。
    pub(in crate::app) fn reveal_log_diff_file(&mut self) {
        let Some(tree) = self.log_diff_tree() else {
            return;
        };
        let height = self.log_diff_list().h;
        let Some(row) = tree
            .rows
            .iter()
            .position(|(_, row)| matches!(row, LogDiffRow::File(index) if *index == tree.selected))
        else {
            return;
        };
        let top = row as f32 * TREE_ROW;
        let scroll = tree.scroll.min(top).max(top + TREE_ROW - height);
        self.scroll_log_diff(scroll);
    }
}
