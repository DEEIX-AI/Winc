//! 从 Git 日志打开的差异所对应的只读文件树。

use std::collections::BTreeMap;
use std::ffi::OsStr;

use super::*;
use crate::layout::SidebarView;
use crate::theme::TREE_ROW;

/// 侧栏中的目录或文件。文件下标对应这次比较的文件列表。
pub(crate) enum LogDiffRow {
    Dir(PathBuf),
    File(usize),
}

/// 差异标签的文件树。展开状态与工作区文件树无关。
pub(crate) struct LogDiffTree {
    pub(super) source: Arc<LogDiffSource>,
    pub(super) selected: usize,
    pub rows: Vec<(usize, LogDiffRow)>,
    pub collapsed: HashSet<PathBuf>,
    pub scroll: f32,
}

impl LogDiffTree {
    /// 标题使用与差异标签相同的提交号。
    pub fn version(&self) -> &str {
        &self.source.version
    }

    pub fn files(&self) -> &[CommitFile] {
        &self.source.files
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        let root = &self.source.root;
        let paths = self
            .source
            .files
            .iter()
            .enumerate()
            .map(|(i, file)| (i, file.path.strip_prefix(root).unwrap_or(&file.path)));
        append_rows(&mut self.rows, &self.collapsed, Path::new(""), 0, paths);
    }
}

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
    for (name, children) in dirs {
        let dir = parent.join(name);
        let open = !collapsed.contains(&dir);
        rows.push((depth, LogDiffRow::Dir(dir.clone())));
        if open {
            append_rows(rows, collapsed, &dir, depth + 1, children);
        }
    }
    files.sort_unstable();
    rows.extend(
        files
            .into_iter()
            .map(|(_, index)| (depth, LogDiffRow::File(index))),
    );
}

impl App {
    /// 返回当前侧栏展示的历史差异文件树。
    pub(crate) fn log_diff_tree(&self) -> Option<&LogDiffTree> {
        let id = self.editor.active()?.id;
        (self.sidebar_view == SidebarView::Commit && self.steps_log_files(id))
            .then_some(self.log.tree.as_ref())
            .flatten()
    }

    pub(crate) fn log_diff_list(&self) -> Rect {
        let panel = self.panel_rect();
        let top = self.layout.sidebar_header.bottom();
        Rect::new(panel.x, top, panel.w, (panel.bottom() - top).max(0.0))
    }

    pub(crate) fn log_diff_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let tree = self.log_diff_tree()?;
        let list = self.log_diff_list();
        if !list.contains(x, y) {
            return None;
        }
        let row = ((y - list.y + tree.scroll) / TREE_ROW) as usize;
        (row < tree.rows.len()).then_some(row)
    }

    pub(in crate::app) fn scroll_log_diff(&mut self, scroll: f32) {
        let height = self.log_diff_list().h;
        if self.log_diff_tree().is_some()
            && let Some(tree) = &mut self.log.tree
        {
            tree.scroll = scroll.clamp(0.0, (tree.rows.len() as f32 * TREE_ROW - height).max(0.0));
        }
    }

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
                self.open_log_diff_file(file);
            }
            Some(LogDiffRow::File(_)) => {
                self.log.diff_request += 1;
                self.diff_land = None;
            }
            _ => {}
        }
        self.request_redraw();
    }

    pub(super) fn open_log_diff_file(&mut self, file: usize) {
        if let Some(tree) = &self.log.tree {
            self.load_log_diff(Arc::clone(&tree.source), file);
        }
    }

    pub(super) fn select_log_diff(&mut self, source: Arc<LogDiffSource>, file: usize) {
        let keep = self
            .log
            .tree
            .as_mut()
            .filter(|tree| Arc::ptr_eq(&tree.source, &source) || tree.source == source);
        let tree = if let Some(tree) = keep {
            tree.selected = file;
            tree
        } else {
            let tree = self.log.tree.insert(LogDiffTree {
                source,
                selected: file,
                rows: Vec::new(),
                collapsed: HashSet::new(),
                scroll: 0.0,
            });
            tree.rebuild();
            tree
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
