//! 提交和回滚：整个文件或只提交一部分改动，提交说明。
//!
//! [`Git::rollback`] 把文件恢复成 `HEAD` 里的内容，[`Git::commit`] 只提交指定的文件，其余
//! 改动留在暂存区和工作区。只提交一部分改动时，先在临时索引里拼出要提交的内容，再提交。

use super::*;

impl Git {
    /// 回滚已跟踪的 `paths`：暂存区和工作区都恢复成 `HEAD` 里的内容（`git restore`）。
    ///
    /// `added` 是 `HEAD` 里没有、新加入版本管理的文件，只从暂存区移除，工作区里的文件由调用方处理。
    ///
    /// # Errors
    ///
    /// `git` 报错时返回它的说明。
    pub fn rollback(
        &self,
        root: &Path,
        paths: &[PathBuf],
        added: &[PathBuf],
    ) -> Result<(), String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        if !added.is_empty() {
            let mut command = self.command(root);
            command
                .args(["rm", "--cached", "--quiet", "--"])
                .args(relative_all(root, added));
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        }
        if !paths.is_empty() {
            let mut command = self.command(root);
            command
                .args(["restore", "--source=HEAD", "--staged", "--worktree", "--"])
                .args(relative_all(root, paths));
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        }
        Ok(())
    }

    /// 提交：只提交 `paths` 指定的文件，暂存区里其他文件的改动原样留着。
    ///
    /// `amend` 为 `true` 时修改上一次提交，`paths` 可以为空，只改提交信息。返回新提交号的前 7 位。
    /// `add` 中未跟踪的文件先 `git add`；`partial` 非空时在临时索引里拼出要提交的内容。
    ///
    /// # Errors
    ///
    /// `git` 报错（hooks 拒绝、没有可提交的内容、身份没配置……）时返回它的说明。
    pub fn commit(
        &self,
        root: &Path,
        message: &str,
        paths: &[PathBuf],
        add: &[PathBuf],
        partial: &[Partial],
        amend: bool,
    ) -> Result<String, String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        // 合并、拣选、还原进行中时不能用 `--only`：勾上的文件先加入暂存区，再提交整个暂存区，
        // 这样才是一个合并提交。
        let operation = git_dir(root).and_then(|d| Operation::detect(&d));
        if let Some(operation @ (Operation::Merge | Operation::CherryPick | Operation::Revert)) =
            operation
        {
            if !partial.is_empty() {
                let op = operation.label();
                let error = winc_i18n::t!("git-commit-partial-during-operation", operation = op);
                return Err(error);
            }
            if !paths.is_empty() {
                let mut command = self.command(root);
                command
                    .args(["add", "--all", "--"])
                    .args(relative_all(root, paths));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }
            let mut command = self.command(root);
            command.args(["commit", "--file=-", "--cleanup=whitespace"]);
            self.run_commit(root, &mut command, message)?;
            return self.short_head(root);
        }
        if !partial.is_empty() {
            return self.commit_partial(root, message, paths, partial, amend);
        }
        if !add.is_empty() {
            let mut command = self.command(root);
            command.args(["add", "--"]).args(relative_all(root, add));
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
        }
        let mut command = self.command(root);
        // `whitespace` 只去掉首尾空行和行尾空白；`strip` 还会删掉 `#` 开头的行，
        // 「#42 修复登录」这样的信息会被整段删除。
        command.args(["commit", "--file=-", "--cleanup=whitespace"]);
        if amend {
            command.arg("--amend");
        }
        command
            .arg("--only")
            .arg("--")
            .args(relative_all(root, paths));
        if let Err(e) = self.run_commit(root, &mut command, message) {
            // 提交被拒绝（hooks、身份没配置……）时把刚加入暂存区的未跟踪文件移出，暂存区回到提交
            // 之前的状态。`rm --cached` 在还没有提交过的仓库里也能用；回退失败仍报提交本身的原因。
            if !add.is_empty() {
                let mut command = self.command(root);
                command
                    .args(["rm", "--cached", "--quiet", "--ignore-unmatch", "--"])
                    .args(relative_all(root, add));
                let _ = self.run(&mut command, STATUS_TIMEOUT, &cancel);
            }
            return Err(e);
        }
        self.short_head(root)
    }

    /// 有只提交一部分的文件时执行的提交，见 [`Git::commit`]。
    fn commit_partial(
        &self,
        root: &Path,
        message: &str,
        paths: &[PathBuf],
        partial: &[Partial],
        amend: bool,
    ) -> Result<String, String> {
        self.with_commit_index(root, paths, partial, |index_arg, _head| {
            let with_index = |command: &mut Command| {
                command.env("GIT_INDEX_FILE", index_arg);
            };
            let cancel = Cancel::default();
            let mut command = self.command(root);
            with_index(&mut command);
            command.args(["commit", "--file=-", "--cleanup=whitespace"]);
            if amend {
                command.arg("--amend");
            }
            self.run_commit(root, &mut command, message)?;

            // 实际的暂存区里，`paths` 换成刚提交的内容。
            if !paths.is_empty() {
                let mut command = self.command(root);
                command
                    .args(["reset", "--quiet", "--"])
                    .args(relative_all(root, paths));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }
            self.short_head(root)
        })
    }

    /// 在独立索引中拼出要提交的树；真实索引和工作区不变。
    fn with_commit_index<T>(
        &self,
        root: &Path,
        paths: &[PathBuf],
        partial: &[Partial],
        work: impl FnOnce(&OsStr, Option<&str>) -> Result<T, String>,
    ) -> Result<T, String> {
        let cancel = Cancel::default();
        let index = crate::wsl::temp_dir(root).join(format!(
            "winc-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        // 发行版内的 `git` 只接受发行版内的路径。
        let index_arg = crate::wsl::locate_here(&index)
            .map_or_else(|| index.clone().into_os_string(), |place| place.path.into());
        let with_index = |command: &mut Command| {
            command.env("GIT_INDEX_FILE", &index_arg);
        };
        let result = (|| {
            // 明确区分 unborn HEAD 与其他失败，并固定本次快照的基线，不能把读取失败当作空树。
            let mut command = self.command(root);
            command
                .args(["rev-parse", "--verify", "--quiet", "HEAD"])
                .stderr(Stdio::piped());
            let head = match process::run(&mut command, Vec::new(), SHOW_TIMEOUT, &cancel) {
                Ok(output) if output.status.success() => {
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
                }
                Ok(output) if output.status.code() == Some(1) => None,
                Ok(output) => return Err(failure(&output.stderr)),
                Err(e) => return Err(describe(&e)),
            };
            let mut command = self.command(root);
            with_index(&mut command);
            command.args(["read-tree", head.as_deref().unwrap_or("--empty")]);
            self.run(&mut command, STATUS_TIMEOUT, &cancel)?;

            let whole: Vec<PathBuf> = paths
                .iter()
                .filter(|p| !partial.iter().any(|q| &q.path == *p))
                .cloned()
                .collect();
            if !whole.is_empty() {
                let mut command = self.command(root);
                with_index(&mut command);
                command
                    .args(["add", "--all", "--"])
                    .args(relative_all(root, &whole));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }
            for part in partial {
                let relative = slash_path(root, &part.path)?;
                // 按工作区的形式写进去：`--path` 套用该路径的过滤器（autocrlf 等）。
                let mut command = self.command(root);
                command
                    .args(["hash-object", "-w", "--stdin"])
                    .arg(joined_arg(&[OsStr::new("--path="), &relative]))
                    .stderr(Stdio::piped());
                let content = part.content.clone();
                let oid = match process::run(&mut command, content, STATUS_TIMEOUT, &cancel) {
                    Ok(output) if output.status.success() => {
                        String::from_utf8_lossy(&output.stdout).trim().to_owned()
                    }
                    Ok(output) => return Err(failure(&output.stderr)),
                    Err(e) => return Err(describe(&e)),
                };
                let mode = self.file_mode(&part.path, &cancel)?;
                let mut command = self.command(root);
                with_index(&mut command);
                command
                    .args(["update-index", "--add", "--cacheinfo"])
                    .arg(joined_arg(&[
                        OsStr::new(&format!("{mode},{oid},")),
                        &relative,
                    ]));
                self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            }

            work(&index_arg, head.as_deref())
        })();
        let _ = std::fs::remove_file(&index);
        result
    }

    /// 读取将要提交的统计和补丁，包含部分勾选、新文件和 amend 后的完整改动。
    ///
    /// 使用临时索引，不改变真实索引；补丁最多读取 `patch_limit` 字节。
    ///
    /// # Errors
    ///
    /// 仓库不受信任、文件无法读取或 git 失败时返回原因。
    pub fn commit_diff(
        &self,
        root: &Path,
        paths: &[PathBuf],
        partial: &[Partial],
        amend: bool,
        patch_limit: usize,
    ) -> Result<(String, String), String> {
        self.require_trust(root)?;
        self.with_commit_index(root, paths, partial, |index_arg, head| {
            let cancel = Cancel::default();
            let base = if amend {
                let head = head.ok_or_else(|| winc_i18n::t!("git-commit-ai-no-head").to_owned())?;
                let mut command = self.command(root);
                command.args(["cat-file", "-p", head]);
                let commit = self.run(&mut command, SHOW_TIMEOUT, &cancel)?;
                // 直接读 commit 头，浅克隆中缺失的父对象必须报错，不能误当根提交。
                String::from_utf8_lossy(&commit)
                    .lines()
                    .take_while(|line| !line.is_empty())
                    .find_map(|line| line.strip_prefix("parent ").map(str::to_owned))
            } else {
                head.map(str::to_owned)
            };
            // 根提交没有父提交：与空树比较，也适用于还没有 HEAD 的仓库。
            let base = match base {
                Some(base) => base,
                None => {
                    let mut command = self.command(root);
                    command.args(["hash-object", "-w", "-t", "tree", "--stdin"]);
                    String::from_utf8_lossy(&self.run(&mut command, SHOW_TIMEOUT, &cancel)?)
                        .trim()
                        .to_owned()
                }
            };
            let diff = || {
                let mut command = self.command(root);
                command.env("GIT_INDEX_FILE", index_arg).args([
                    "diff",
                    "--cached",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                ]);
                command
            };
            let mut command = diff();
            command.args(["--stat=160,120", &base]);
            let stat = self.run(&mut command, STATUS_TIMEOUT, &cancel)?;
            let mut command = diff();
            command.args(["-p", "--unified=3", &base, "--", "."]);
            // 这些文件仍出现在概览里，避免生成产物挤占源码的补丁预算。
            for pattern in [
                "Cargo.lock",
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
                "*.min.js",
                "*.min.css",
            ] {
                command.arg(format!(":(glob,exclude)**/{pattern}"));
            }
            let mut stream = process::Stream::spawn(&mut command).map_err(|e| describe(&e))?;
            let (patch, truncated) =
                stream
                    .read_prefix(patch_limit, STATUS_TIMEOUT)
                    .map_err(|e| match e {
                        process::StreamError::Run(e) => describe(&e),
                        process::StreamError::Exited(stderr) => failure(&stderr),
                    })?;
            let mut patch = String::from_utf8_lossy(&patch).into_owned();
            if truncated {
                patch.push_str("\n[Further diff omitted; see file summary]\n");
            }
            Ok((String::from_utf8_lossy(&stat).into_owned(), patch))
        })
    }

    /// 运行 `git commit`，提交信息由标准输入传入。
    fn run_commit(&self, root: &Path, command: &mut Command, message: &str) -> Result<(), String> {
        command.stderr(Stdio::piped());
        match process::run(
            command,
            message.as_bytes().to_vec(),
            COMMIT_TIMEOUT,
            &Cancel::default(),
        ) {
            Ok(output) => {
                // hooks 的输出可能在标准输出里。
                let mut text = output.stderr;
                text.extend_from_slice(&output.stdout);
                output::record(root, &["commit"], output.status.code(), &text);
                if output.status.success() {
                    Ok(())
                } else {
                    Err(failure(&text))
                }
            }
            Err(e) => {
                let reason = describe(&e);
                output::record(root, &["commit"], None, reason.as_bytes());
                Err(reason)
            }
        }
    }

    /// 返回 `HEAD` 的提交号（前 7 位）。
    fn short_head(&self, root: &Path) -> Result<String, String> {
        let mut command = self.command(root);
        command.args(["rev-parse", "--short", "HEAD"]);
        let oid = self.run(&mut command, SHOW_TIMEOUT, &Cancel::default())?;
        Ok(String::from_utf8_lossy(&oid).trim().to_owned())
    }

    /// 返回 `HEAD` 指向的提交；还没有提交过、`git` 运行失败时为 `None`。
    pub fn last_commit(&self, root: &Path) -> Option<LastCommit> {
        let mut command = self.command(root);
        command.args([
            "log",
            "-1",
            "--date=format-local:%Y-%m-%d %H:%M",
            "--format=%H%x00%ct%x00%ad%x00%B",
        ]);
        let stdout = self
            .run(&mut command, SHOW_TIMEOUT, &Cancel::default())
            .ok()?;
        let text = String::from_utf8_lossy(&stdout);
        let mut fields = text.splitn(4, '\0');
        let oid = fields.next()?.trim().to_owned();
        let time = fields.next()?.trim().parse().ok()?;
        let local = fields.next()?.to_owned();
        let message = fields.next()?.trim_end().to_owned();
        let subject = message.lines().next().unwrap_or_default().to_owned();
        (!oid.is_empty()).then_some(LastCommit {
            oid,
            subject,
            message,
            time,
            local,
        })
    }

    /// 撤销 `HEAD` 指向的提交 `oid`：分支退回上一个提交，这次提交的改动留在暂存区，工作区不动
    /// （同 `git reset --soft HEAD~1`）；`oid` 是第一个提交时删掉分支的引用，改动同样留在暂存区。
    ///
    /// 只在分支仍指向 `oid` 时改动（`update-ref` 带旧值），之后又提交过、换了分支时不撤销。
    ///
    /// # Errors
    ///
    /// 仓库不受信任、`HEAD` 已经不是 `oid`、`git` 报错时返回说明。
    pub fn undo_commit(&self, root: &Path, oid: &str) -> Result<(), String> {
        self.require_trust(root)?;
        let cancel = Cancel::default();
        let mut command = self.command(root);
        command.args(["rev-parse", "--verify", "--quiet", &format!("{oid}^")]);
        let parent = self.run(&mut command, SHOW_TIMEOUT, &cancel).ok();
        let parent = parent.map(|p| String::from_utf8_lossy(&p).trim().to_owned());
        let mut command = self.command(root);
        command.args(["update-ref", "-m", "reset: undo commit"]);
        match parent.filter(|p| !p.is_empty()) {
            Some(parent) => command.args(["HEAD", &parent, oid]),
            None => command.args(["-d", "HEAD", oid]),
        };
        self.run(&mut command, SHOW_TIMEOUT, &cancel)?;
        Ok(())
    }

    /// 返回上一次提交的完整提交信息；还没有提交过、`git` 运行失败时为空字符串。
    pub fn last_message(&self, root: &Path) -> String {
        let mut command = self.command(root);
        command.args(["log", "-1", "--format=%B"]);
        match self.run(&mut command, SHOW_TIMEOUT, &Cancel::default()) {
            Ok(stdout) => String::from_utf8_lossy(&stdout).trim_end().to_owned(),
            Err(_) => String::new(),
        }
    }
}
