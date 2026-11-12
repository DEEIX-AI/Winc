//! 改动文件的操作（新建、改名、移动、删除）在哪个线程上执行。
//!
//! 本机的文件就地执行，结果立即处理。远端主机上的文件要等待网络，界面线程不发出远端的请求
//! （见 [`winc_backend::remote::forbid_blocking`]）：访问磁盘的部分交给后台线程，完成后回到界面
//! 线程处理结果、更新文件树。等待期间界面照常响应。

use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;

use crate::app::{App, UserEvent};

/// 后台执行的结果，类型由发起时的两个闭包约定。
type Outcome = Box<dyn Any + Send>;

/// 一个操作的编号和它的结果。
type Done = (u64, Outcome);

/// 结果回到界面线程后的处理。
type Then = Box<dyn FnOnce(&mut App, Outcome)>;

/// 正在后台执行的文件操作。
pub(in crate::app) struct FileWork {
    /// 下一个操作的编号。
    next: u64,
    /// 各操作完成后的处理，按编号取用。
    pending: HashMap<u64, Then>,
    results: (mpsc::Sender<Done>, mpsc::Receiver<Done>),
}

impl Default for FileWork {
    fn default() -> Self {
        Self {
            next: 0,
            pending: HashMap::new(),
            results: mpsc::channel(),
        }
    }
}

impl App {
    /// 执行访问磁盘的 `work`，再在界面线程上以其结果调用 `then`。
    ///
    /// `at` 是操作涉及的路径：在本机时就地执行，`then` 在返回之前调用；在远端主机上时 `work`
    /// 在后台线程上执行，`then` 稍后由 [`App::poll_file_work`] 调用。
    pub(in crate::app) fn file_work<T: Send + 'static>(
        &mut self,
        at: &Path,
        work: impl FnOnce() -> T + Send + 'static,
        then: impl FnOnce(&mut App, T) + 'static,
    ) {
        if !winc_backend::is_remote(at) {
            let outcome = work();
            then(self, outcome);
            return;
        }
        let id = self.file_work.next;
        self.file_work.next += 1;
        let then: Then = Box::new(move |app, outcome| {
            // 结果的类型就是 `work` 的返回类型，两者在同一次调用中给出。
            if let Ok(outcome) = outcome.downcast::<T>() {
                then(app, *outcome);
            }
        });
        self.file_work.pending.insert(id, then);
        let sender = self.file_work.results.0.clone();
        self.spawn_then(UserEvent::FileWorked, move || {
            let outcome: Outcome = Box::new(work());
            let _ = sender.send((id, outcome));
        });
    }

    /// 处理后台完成的文件操作。
    pub(in crate::app) fn poll_file_work(&mut self) {
        while let Ok((id, outcome)) = self.file_work.results.1.try_recv() {
            if let Some(then) = self.file_work.pending.remove(&id) {
                then(self, outcome);
            }
        }
    }
}
