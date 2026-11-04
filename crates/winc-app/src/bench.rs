//! 测量钩子：设置环境变量 `WINC_BENCH` 后启用，供 `cargo xtask bench` 使用。
//!
//! 环境变量的值决定测量模式：`startup` 在首帧呈现后退出，`scroll` 连续滚动并统计帧耗时，
//! `search:<内容>`、`picker` 做完后保持运行，`find:<内容>`、`highlight` 输出耗时、帧数和 CPU
//! 时间后退出，其他值只输出时间戳并保持运行。保持运行的模式由外部进程采样内存；未设置时不测量。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 滚动测量采集的帧数（帧）。
///
/// 无外部出处与硬性约束，取自调参：按 60 Hz 刷新率计，该帧数对应约 5 秒的滚动时长，
/// 使 p50、p95 与最大耗时的统计覆盖数百帧。采集满该帧数后输出统计结果并退出。
const SCROLL_FRAMES: usize = 300;

/// 测量模式，由环境变量 `WINC_BENCH` 决定。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// 不测量。
    Off,
    /// 输出启动时间戳后保持运行，供外部进程采样内存。
    Idle,
    /// 首帧呈现后退出，用于测量启动耗时。
    Startup,
    /// 连续滚动并统计每帧耗时，采集 [`SCROLL_FRAMES`] 帧后退出。
    Scroll,
    /// 首帧呈现后在打开的目录里搜索给定的内容，之后保持运行。
    Search(String),
    /// 首帧呈现后打开快速打开面板，之后保持运行。
    QuickOpen,
    /// 首帧呈现后在打开的文件里查找给定的内容，搜到末尾后退出。
    Find(String),
    /// 首帧呈现后跳到文件末尾，高亮推算完后退出。
    Highlight,
}

/// 每帧呈现之后要执行的动作。
pub enum Next {
    /// 不做任何事，继续正常运行。
    Continue,
    /// 滚动测量：调用方向下滚动一段并请求下一帧。
    Scroll,
    /// 测量完成，退出事件循环。
    Exit,
    /// 在打开的目录里搜索给定的内容。
    Search(String),
    /// 打开快速打开面板。
    QuickOpen,
    /// 在打开的文件里查找给定的内容。
    Find(String),
    /// 跳到文件末尾。
    Highlight,
}

/// 测量钩子的状态。
pub struct Bench {
    mode: Mode,
    /// 滚动测量中已采集的每帧耗时；只在 [`Mode::Scroll`] 下使用。
    frame_times: Vec<Duration>,
    /// 查找、高亮测量开始的时刻、进程已用的 CPU 时间和之后画了几帧。
    progress: Option<(Instant, Duration, usize)>,
}

impl Bench {
    /// 创建一个不测量的状态，供后续新建的窗口使用。
    ///
    /// 测量只针对启动时的窗口。
    pub fn off() -> Self {
        Self {
            mode: Mode::Off,
            frame_times: Vec::new(),
            progress: None,
        }
    }

    /// 按环境变量 `WINC_BENCH` 创建测量状态；变量未设置时不测量。
    pub fn from_env() -> Self {
        let mode = match std::env::var("WINC_BENCH").as_deref() {
            Err(_) => Mode::Off,
            Ok("startup") => Mode::Startup,
            Ok("scroll") => Mode::Scroll,
            Ok("picker") => Mode::QuickOpen,
            Ok("highlight") => Mode::Highlight,
            Ok(value) => match (value.strip_prefix("search:"), value.strip_prefix("find:")) {
                (Some(query), _) => Mode::Search(query.to_owned()),
                (_, Some(query)) => Mode::Find(query.to_owned()),
                _ => Mode::Idle,
            },
        };

        Self {
            mode,
            frame_times: Vec::new(),
            progress: None,
        }
    }

    /// 是否启用测量模式。
    ///
    /// 测量模式下窗口静默：不激活应用、不出现在程序坞、窗口完全透明且鼠标点击穿透。窗口仍然
    /// 上屏并呈现，因此测得的绘制与呈现耗时与正常运行时相同，仅是不显示、不响应点击。
    ///
    /// 测量时窗口静默：不激活应用、不出现在程序坞、窗口全透明且鼠标点击穿透。
    ///
    /// 窗口仍然实际上屏并呈现，因此测到的绘制和呈现耗时与正常运行时相同；只是用户看不见、点不到。
    pub fn silent(&self) -> bool {
        self.mode != Mode::Off
    }

    /// 输出进程入口的时间戳（Unix 纪元起的毫秒数），由 `main` 的入口处调用；不测量时不输出。
    pub fn on_start(&self) {
        if self.mode != Mode::Off {
            println!("WINC main_entry_epoch_ms={:.3}", epoch_ms());
        }
    }

    /// 更新呈现计数与帧耗时，返回下一帧要执行的动作。
    ///
    /// 每次呈现之后调用。
    ///
    /// `presented` 是包括当前帧在内的累计呈现次数，从 1 起；第 1 帧画在尚未显示的窗口上，
    /// 因此以第 2 帧作为首次呈现，滚动测量也从第 3 帧起采样。`elapsed` 是当前帧的绘制与呈现耗时。
    pub fn on_present(&mut self, presented: usize, elapsed: Duration) -> Next {
        if self.mode == Mode::Off {
            return Next::Continue;
        }

        if presented == 2 {
            println!("WINC first_present_epoch_ms={:.3}", epoch_ms());

            match &self.mode {
                Mode::Startup => return Next::Exit,
                Mode::Search(query) => return Next::Search(query.clone()),
                Mode::QuickOpen => return Next::QuickOpen,
                Mode::Find(query) => {
                    self.progress = Some((Instant::now(), cpu_time(), 0));
                    return Next::Find(query.clone());
                }
                Mode::Highlight => {
                    self.progress = Some((Instant::now(), cpu_time(), 0));
                    return Next::Highlight;
                }
                Mode::Off | Mode::Idle | Mode::Scroll => {}
            }
        }

        if self.mode != Mode::Scroll || presented < 2 {
            return Next::Continue;
        }

        if presented > 2 {
            self.frame_times.push(elapsed);
        }

        if self.frame_times.len() < SCROLL_FRAMES {
            return Next::Scroll;
        }

        self.frame_times.sort_unstable();
        let pick = |q: f64| {
            self.frame_times[((self.frame_times.len() - 1) as f64 * q) as usize].as_secs_f64()
                * 1000.0
        };
        println!(
            "WINC scroll_frames={} p50_ms={:.2} p95_ms={:.2} max_ms={:.2}",
            self.frame_times.len(),
            pick(0.5),
            pick(0.95),
            pick(1.0)
        );
        Next::Exit
    }
}

impl Bench {
    /// 记录一帧进度，返回测量是否已完成。
    ///
    /// 查找、高亮测量中每画完一帧调用；`done` 表示目标动作是否完成。完成时输出从开始到现在的
    /// 耗时、累计帧数和这段时间进程用掉的 CPU 时间，返回 `true`，调用方应退出事件循环。
    pub fn on_progress(&mut self, done: bool) -> bool {
        let Some((started, cpu, frames)) = &mut self.progress else {
            return false;
        };
        *frames += 1;
        // 开始测量的那一帧里刚发出查找、跳转，`done` 还是上一帧的结论。
        if *frames == 1 || !done {
            return false;
        }
        println!(
            "WINC progress_ms={:.1} progress_frames={} progress_cpu_ms={:.1}",
            started.elapsed().as_secs_f64() * 1000.0,
            frames,
            cpu_time().saturating_sub(*cpu).as_secs_f64() * 1000.0
        );
        true
    }
}

/// 返回进程到现在用掉的 CPU 时间（用户态加内核态，所有线程）。
#[allow(unsafe_code)] // getrusage
fn cpu_time() -> Duration {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: `getrusage` 只写入传入的结构体。
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            // SAFETY: 调用成功时结构体已填好。
            let usage = unsafe { usage.assume_init() };
            let time = |t: libc::timeval| {
                Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64)
            };
            return time(usage.ru_utime) + time(usage.ru_stime);
        }
    }
    Duration::ZERO
}

/// 返回当前时间距 Unix 纪元的毫秒数；系统时钟早于纪元时返回 0。
fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}
