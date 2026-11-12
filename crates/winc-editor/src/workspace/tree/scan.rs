//! 在后台重读文件树里被其他程序改过的目录。
//!
//! 文件监视通知目录变了以后，界面线程上只挑出要重读哪些目录（[`Workspace::scan`]），读目录
//! 交给后台（[`DirScan::run`]），读好后再换进缓存、重新拍平（[`Workspace::apply_scan`]）：大
//! 目录读一遍要十几毫秒，网络盘上可能长时间阻塞。

use super::*;

/// 要在后台重读的目录，见 [`Workspace::scan`]。
#[derive(Debug)]
pub struct DirScan {
    root: PathBuf,
    dirs: Vec<PathBuf>,
    expanded: Vec<PathBuf>,
}

/// 后台读好的目录内容，见 [`DirScan::run`]，交给 [`Workspace::apply_scan`]。
#[derive(Debug)]
pub struct DirListing {
    root: PathBuf,
    /// 各目录读到的内容；读不出来（目录已不存在）时为 `None`。
    dirs: Vec<(PathBuf, Option<Vec<DirEntry>>)>,
    /// 已展开、但已经不是目录的路径。
    gone: Vec<PathBuf>,
}

impl DirScan {
    /// 要重读的目录，按读的顺序；[`DirListing::retain`] 按同样的顺序给出。
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// 读各目录，并检查已展开的目录还是不是目录（在后台线程上调用）。
    pub fn run(self) -> DirListing {
        DirListing {
            dirs: self
                .dirs
                .into_iter()
                .map(|dir| {
                    let fresh = list_dir(&dir).ok();
                    (dir, fresh)
                })
                .collect(),
            gone: self
                .expanded
                .into_iter()
                .filter(|p| !winc_backend::fs::is_dir(p))
                .collect(),
            root: self.root,
        }
    }
}

impl DirListing {
    /// 只留下 `keep` 返回 `true` 的目录（按 [`DirScan::dirs`] 的顺序逐个询问），用来丢弃
    /// 期间又被重读过的、已经过时的目录。
    pub fn retain(&mut self, mut keep: impl FnMut(&Path) -> bool) {
        self.dirs.retain(|(dir, _)| keep(dir));
    }
}

impl Workspace {
    /// 返回 `paths` 在磁盘上发生变化（被其他程序新建、删除或改名）以后要重读的目录：已缓存且
    /// 受影响的目录（路径所在的目录，以及路径本身是目录时的该目录）。
    ///
    /// `paths` 为 `None`（整个重新扫描）时是全部已缓存的目录。
    pub fn scan(&self, paths: Option<&[PathBuf]>) -> DirScan {
        let mut dirs: Vec<PathBuf> = match paths {
            Some(paths) => {
                let mut dirs: HashSet<&Path> = HashSet::new();
                for path in paths {
                    dirs.extend(path.parent());
                    dirs.insert(path);
                }
                dirs.into_iter()
                    .filter(|d| self.children.contains_key(*d))
                    .map(Path::to_path_buf)
                    .collect()
            }
            None => self.children.keys().cloned().collect(),
        };
        dirs.sort();
        DirScan {
            root: self.root.clone(),
            dirs,
            expanded: self.expanded.iter().cloned().collect(),
        }
    }

    /// 换上后台读好的目录内容 `listing`；返回文件树是否发生了变化。
    ///
    /// 目录内容确实变化时才重新拍平，这时同时收起已经不是目录的展开项。选中项保持不变。读的是
    /// 另一个工作区（期间换了目录）时什么也不做。
    pub fn apply_scan(&mut self, listing: DirListing) -> bool {
        if listing.root != self.root {
            return false;
        }
        let mut changed = false;
        for (dir, fresh) in listing.dirs {
            let Some(cached) = self.children.get(&dir) else {
                continue;
            };
            let Some(fresh) = fresh else {
                // 目录已不存在：其上级目录重读时不再包含它，同时丢弃它的缓存。
                self.children.remove(&dir);
                changed = true;
                continue;
            };
            let same = cached.len() == fresh.len()
                && cached
                    .iter()
                    .zip(&fresh)
                    .all(|(a, b)| a.path == b.path && a.kind == b.kind);
            if !same {
                self.children.insert(dir, fresh.into());
                changed = true;
            }
        }

        if changed {
            for path in &listing.gone {
                self.expanded.remove(path);
            }
            self.rebuild();
        }
        changed
    }
}
