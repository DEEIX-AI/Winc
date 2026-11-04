//! 遍历目录：多个线程并行取文件并搜索，结果送回界面线程。

use super::*;

impl ProjectSearch {
    /// 按当前的匹配器在后台搜索 `source` 中的文件，取代正在进行的搜索（旧的随之停止）。
    ///
    /// `refresh` 为 `None` 时是整体搜索，否则是重新搜索一批文件。
    pub(super) fn launch(
        &mut self,
        source: Source,
        dirty: Arc<HashMap<PathBuf, Rope>>,
        refresh: Option<Refresh>,
    ) {
        let (Some(matcher), Some(wake)) = (&self.matcher, &self.wake) else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let job = Job {
            cancel: Arc::new(AtomicBool::new(false)),
            truncated: Arc::new(AtomicBool::new(false)),
            receiver,
            refresh,
        };
        let walk = Walk {
            root: self.root.clone(),
            matcher: matcher.clone(),
            source,
            dirty,
            cancel: Arc::clone(&job.cancel),
            truncated: Arc::clone(&job.truncated),
            count: Arc::new(AtomicUsize::new(0)),
            wake: Arc::clone(wake),
            last_wake: Arc::new(Mutex::new(None)),
        };
        crate::background::spawn(move || Arc::new(walk).run(sender));
        self.job = Some(job);
    }
}

/// 后台搜索哪些文件。
pub(super) enum Source {
    /// 遍历根目录下的全部文件，按「包含」「排除」规则过滤（整体搜索）。
    Tree(Override),
    /// 只搜索这些文件（重新搜索一批文件）。
    Files(Vec<PathBuf>),
}

/// 一次搜索的后台部分。
pub(super) struct Walk {
    root: PathBuf,
    matcher: Matcher,
    source: Source,
    dirty: Arc<HashMap<PathBuf, Rope>>,
    cancel: Arc<AtomicBool>,
    truncated: Arc<AtomicBool>,
    /// 已记录的匹配数，由多个线程共用。
    count: Arc<AtomicUsize>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// 上一次唤醒界面线程的时刻；为 `None`（第一批结果）时立即唤醒。
    last_wake: Arc<Mutex<Option<Instant>>>,
}

impl Walk {
    /// 一个线程遍历目录（或给定的文件列表），把要搜索的文件放进队列；另有若干后台线程从队列取
    /// 文件读取并匹配，遍历结束后遍历线程也参与。
    ///
    /// 文件读取和匹配是主要开销，因此由多个线程并行处理。
    pub(super) fn run(self: Arc<Self>, sender: Sender<Message>) {
        let threads = thread::available_parallelism()
            .map_or(2, |n| n.get())
            .min(MAX_THREADS);
        let queue = Arc::new(Queue::default());

        let helpers = {
            let (walk, queue, sender) = (Arc::clone(&self), Arc::clone(&queue), sender.clone());
            crate::background::crew(threads - 1, move || {
                let mut worker = Worker::new(&walk, sender.clone());
                while let Some(path) = queue.pop() {
                    if worker.visit(&path) == WalkState::Quit {
                        walk.cancel.store(true, Relaxed);
                    }
                }
            })
        };

        let paths: Box<dyn Iterator<Item = PathBuf> + '_> = match &self.source {
            Source::Tree(overrides) => Box::new(
                walker(&self.root)
                    .overrides(overrides.clone())
                    .build()
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_type()
                            .is_some_and(|t| is_listed_file(t, entry.path()))
                    })
                    .map(ignore::DirEntry::into_path),
            ),
            Source::Files(files) => Box::new(files.iter().cloned()),
        };
        let mut worker = Worker::new(&self, sender.clone());
        for path in paths {
            if self.cancel.load(Relaxed) {
                break;
            }
            if let Some(path) = queue.push(path)
                && worker.visit(&path) == WalkState::Quit
            {
                self.cancel.store(true, Relaxed);
            }
        }
        queue.close();

        // 遍历结束后，当前线程也从队列取文件处理。
        while let Some(path) = queue.pop() {
            if worker.visit(&path) == WalkState::Quit {
                self.cancel.store(true, Relaxed);
            }
        }
        drop(worker);
        helpers.wait();

        let _ = sender.send(Message::Done);
        (self.wake)();
    }

    /// 结果送出后调用：距上次唤醒超过 [`WAKE_INTERVAL`] 时才唤醒界面线程。
    fn maybe_wake(&self) {
        let mut last = self
            .last_wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        if last.is_none_or(|at| at.elapsed() >= WAKE_INTERVAL) {
            *last = Some(Instant::now());
            drop(last);
            (self.wake)();
        }
    }
}

/// 遍历线程放入、读取线程取走的待搜索文件队列。
///
/// 遍历结束后关闭队列，取空后读取线程退出。
#[derive(Default)]
struct Queue {
    state: Mutex<(std::collections::VecDeque<PathBuf>, bool)>,
    ready: std::sync::Condvar,
}

/// 队列中待搜索文件的最大数量（个，经验值）。
///
/// 遍历目录比读文件快，不加限制时整个目录的路径都会堆积在队列里；该值只影响队列的内存占用。
/// 队列已满时的处理见 [`Queue::push`]。
const QUEUE_MAX: usize = 1024;

impl Queue {
    /// 把 `path` 放入队列；队列已满时取出最早的一个返回，由遍历线程自己处理。
    ///
    /// 这样即使帮手线程都在忙，遍历线程也总有文件可处理。
    fn push(&self, path: PathBuf) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let full = (state.0.len() >= QUEUE_MAX)
            .then(|| state.0.pop_front())
            .flatten();
        state.0.push_back(path);
        self.ready.notify_one();
        full
    }

    /// 关闭队列：不再放入新文件，取空后 [`Queue::pop`] 返回 `None`。
    fn close(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).1 = true;
        self.ready.notify_all();
    }

    /// 取出一个文件；队列为空时阻塞等待，队列已关闭且取空时返回 `None`。
    fn pop(&self) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(path) = state.0.pop_front() {
                return Some(path);
            }
            if state.1 {
                return None;
            }
            state = self
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// 读文件的线程之一：读取文件、匹配，有结果就发回界面线程。
struct Worker<'a> {
    walk: &'a Walk,
    sender: Sender<Message>,
    /// 每个线程各一个：正则的每个副本有自己的缓存，多个线程共用一个时会争用缓存。
    matcher: Matcher,
    /// 读文件的缓冲，同一线程复用。
    buffer: Vec<u8>,
}

impl<'a> Worker<'a> {
    fn new(walk: &'a Walk, sender: Sender<Message>) -> Self {
        Worker {
            walk,
            sender,
            matcher: walk.matcher.clone(),
            buffer: Vec::new(),
        }
    }

    fn visit(&mut self, path: &Path) -> WalkState {
        if self.walk.cancel.load(Relaxed) {
            return WalkState::Quit;
        }

        let file = match self.walk.dirty.get(path) {
            Some(rope) => self.search_rope(path, rope),
            None => self.search_file(path),
        };

        let Some(mut file) = file else {
            return WalkState::Continue;
        };

        // 多个线程共同计数：使总数超过上限的线程截去多出的部分，并通知所有线程停止。
        let before = self.walk.count.fetch_add(file.matches.len(), Relaxed);

        if before >= MAX_RESULTS {
            self.walk.truncated.store(true, Relaxed);
            return WalkState::Quit;
        }

        let full = before + file.matches.len() >= MAX_RESULTS;
        file.matches.truncate(MAX_RESULTS - before);

        if self.sender.send(Message::File(file)).is_err() {
            return WalkState::Quit;
        }

        self.walk.maybe_wake();

        if full {
            self.walk.truncated.store(true, Relaxed);
            return WalkState::Quit;
        }

        WalkState::Continue
    }

    fn search_file(&mut self, path: &Path) -> Option<FileResult> {
        let limit = self.remaining()?;
        let (matches, lossy) = search_disk(
            &self.matcher,
            &mut self.buffer,
            path,
            limit,
            &self.walk.cancel,
        )?;
        file_result(&self.walk.root, path, matches, lossy)
    }

    fn search_rope(&mut self, path: &Path, rope: &Rope) -> Option<FileResult> {
        let limit = self.remaining()?;
        let matches = search_lines(&self.matcher, rope, limit);
        file_result(&self.walk.root, path, matches, false)
    }

    /// 返回离 [`MAX_RESULTS`] 还差的匹配数（处）；已经达到上限时返回 `None`。
    ///
    /// 单个文件收满该数量之后不必再往下找：单个文件（例如压缩后的大文件）可能含远多于上限的匹配，
    /// 每处还带一段预览。
    fn remaining(&self) -> Option<usize> {
        let left = MAX_RESULTS.saturating_sub(self.walk.count.load(Relaxed));
        (left > 0).then_some(left)
    }
}
