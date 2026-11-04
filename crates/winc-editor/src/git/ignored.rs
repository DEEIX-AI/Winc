//! 按 `.gitignore` 判断文件是否被忽略。

use super::*;

/// 按仓库里的忽略规则判断路径是否被忽略。
///
/// 规则取自文件所在目录到仓库根目录的各级 `.gitignore`（近的优先，`!` 规则可以重新包含上层
/// 忽略的路径）和 `.git/info/exclude`。判断结果只决定目录里的变化要不要刷新状态，不读取全局
/// 忽略规则，漏判只会多刷新一次。读过的文件按目录缓存，`.gitignore` 变化后调用
/// [`Ignored::forget`]。
#[derive(Debug)]
pub struct Ignored {
    root: PathBuf,
    /// 各目录的 `.gitignore`；`None` 表示该目录下没有这个文件。
    cache: HashMap<PathBuf, Option<ignore::gitignore::Gitignore>>,
    /// `.git/info/exclude`；外层 `None` 表示还没读过，内层 `None` 表示没有该文件。
    exclude: Option<Option<ignore::gitignore::Gitignore>>,
    /// 判断过的目录是否被忽略。
    dirs: HashMap<PathBuf, bool>,
}

impl Ignored {
    /// 创建仓库 `root` 的忽略规则，第一次判断时才读取文件。
    pub fn new(root: &Path) -> Ignored {
        Ignored {
            root: root.to_path_buf(),
            cache: HashMap::new(),
            exclude: None,
            dirs: HashMap::new(),
        }
    }

    /// 丢弃读过的规则（`.gitignore` 变化后调用）。
    pub fn forget(&mut self) {
        self.cache.clear();
        self.exclude = None;
        self.dirs.clear();
    }

    /// 返回 `path` 是否被忽略；不在仓库里的路径视为被忽略。
    pub fn is_ignored(&mut self, path: &Path) -> bool {
        if !path.starts_with(&self.root) {
            return true;
        }
        // 所在目录被忽略时，其下的路径都被忽略（`git` 也不会重新包含被忽略目录里的文件）。结论
        // 按目录记住：编译时一批几千个变化大多落在同几个目录，每个目录只判断一次。
        if let Some(parent) = path.parent()
            && parent != self.root
            && self.dir_ignored(parent)
        {
            return true;
        }
        // 不查询 `path` 是不是目录（每个路径多一次系统调用）：按文件判断，目录专用规则漏判只会
        // 多刷新一次。
        self.matches(path, false)
    }

    /// 返回目录 `dir` 是否被忽略（它自己或上面的某一级被忽略），结论按目录缓存。
    fn dir_ignored(&mut self, dir: &Path) -> bool {
        if let Some(&ignored) = self.dirs.get(dir) {
            return ignored;
        }
        let ignored = match dir.parent() {
            Some(parent) if parent != self.root && parent.starts_with(&self.root) => {
                // 上级已被忽略时不记录这一级：编译时 `target/` 下不断新建的目录不会使缓存持续增长，
                // 缓存条目数只与未被忽略的目录有关。
                if self.dir_ignored(parent) {
                    return true;
                }
                self.matches(dir, true)
            }
            _ => self.matches(dir, true),
        };
        self.dirs.insert(dir.to_path_buf(), ignored);
        ignored
    }

    /// 按各级 `.gitignore`（近的优先）和 `.git/info/exclude` 判断 `path` 本身。
    fn matches(&mut self, path: &Path, is_dir: bool) -> bool {
        use ignore::Match;
        let load = |dir: &Path, file: &Path| {
            if !file.is_file() {
                return None;
            }
            let mut builder = ignore::gitignore::GitignoreBuilder::new(dir);
            builder.add(file);
            builder.build().ok()
        };
        for dir in path.ancestors().skip(1) {
            if !dir.starts_with(&self.root) {
                break;
            }
            if !self.cache.contains_key(dir) {
                let matcher = load(dir, &dir.join(".gitignore"));
                self.cache.insert(dir.to_path_buf(), matcher);
            }
            if let Some(Some(matcher)) = self.cache.get(dir) {
                match matcher.matched(path, is_dir) {
                    Match::Ignore(_) => return true,
                    Match::Whitelist(_) => return false,
                    Match::None => {}
                }
            }
        }
        if self.exclude.is_none() {
            let root = self.root.clone();
            self.exclude = Some(load(&root, &root.join(".git/info/exclude")));
        }
        self.exclude
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|m| m.matched(path, is_dir).is_ignore())
    }
}
