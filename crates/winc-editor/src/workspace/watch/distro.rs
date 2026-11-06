//! 发行版内的监视程序 `winc-watch`：启动、接收它报告的变化，以及另外监视公共 Git 目录。
//!
//! 消息格式见 `winc-watch` 的说明。程序的标准输入结束时自行退出，因此 winc 异常退出时不会留下进程。

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use super::Pending;

/// 运行中的监视程序。释放时结束它。
pub(super) struct Helper {
    child: Child,
    stdin: ChildStdin,
    /// 程序是否仍在报告变化。
    alive: Arc<AtomicBool>,
}

impl Helper {
    /// 运行 `command`，在后台线程上把报告的路径（经 `to_local` 转换）记入 `pending`。
    ///
    /// `command` 只需设置程序和参数。程序意外结束时要求重新扫描一次，之后 [`Helper::alive`] 为假。
    ///
    /// # Errors
    ///
    /// `command` 无法启动时返回错误。
    pub(super) fn spawn(
        mut command: Command,
        to_local: impl Fn(&str) -> PathBuf + Send + 'static,
        record: impl Fn(Report) + Send + 'static,
    ) -> io::Result<Helper> {
        // Windows 上图形程序启动命令行程序时默认弹出控制台窗口：`CREATE_NO_WINDOW`（0x0800_0000）
        // 可避免这一点。
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin 是管道");
        let stdout = child.stdout.take().expect("stdout 是管道");
        let alive = Arc::new(AtomicBool::new(true));
        let running = Arc::clone(&alive);
        thread::Builder::new()
            .name("winc-watch 读".into())
            .spawn(move || {
                for message in BufReader::new(stdout).split(0).map_while(Result::ok) {
                    if let Some(report) = parse(&message, &to_local) {
                        record(report);
                    }
                }
                // 由 `Helper` 释放时已经置假，不再报告。
                if running.swap(false, Ordering::Relaxed) {
                    record(Report::Rescan);
                }
            })?;
        Ok(Helper {
            child,
            stdin,
            alive,
        })
    }

    /// 程序是否仍在报告变化。
    pub(super) fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// 让程序另外监视发行版内的目录 `path`。
    pub(super) fn watch(&mut self, path: &str) {
        self.send(b'W', path);
    }

    /// 让程序不再监视发行版内的目录 `path`。
    pub(super) fn unwatch(&mut self, path: &str) {
        self.send(b'U', path);
    }

    /// 发送一条命令；程序已退出时什么也不做，退出由读线程发现。
    fn send(&mut self, kind: u8, path: &str) {
        let mut message = vec![kind];
        message.extend_from_slice(path.as_bytes());
        message.push(0);
        let _ = self
            .stdin
            .write_all(&message)
            .and_then(|()| self.stdin.flush());
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 监视程序报告的一项内容。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Report {
    /// 路径发生变化。
    Path(PathBuf),
    /// 需要重新扫描整个目录。
    Rescan,
}

/// 解析一条不含结尾 NUL 的消息；不需要处理的消息返回 `None`。
fn parse(message: &[u8], to_local: &impl Fn(&str) -> PathBuf) -> Option<Report> {
    match message.split_first()? {
        (b'P', path) => Some(Report::Path(to_local(&String::from_utf8_lossy(path)))),
        (b'R', _) => Some(Report::Rescan),
        _ => None,
    }
}

/// 把 `report` 记入 `pending`，需要唤醒界面线程时返回 `true`。
pub(super) fn apply(pending: &Mutex<Pending>, base: &std::path::Path, report: Report) -> bool {
    let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
    match report {
        Report::Path(path) => pending.add_paths(base, [path]),
        Report::Rescan => pending.rescan = true,
    }
    pending.wake_due()
}
