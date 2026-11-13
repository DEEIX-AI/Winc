//! 应用和窗口的生命周期。
//!
//! 启动、事件循环、窗口和绘制、外观、提示条、关闭和退出、热退出的备份、临时打开模式。

use super::*;

pub(super) mod alert;
pub(super) mod appearance;
pub(super) mod backup;
pub(super) mod closing;
pub(super) mod events;
#[cfg(target_os = "macos")]
mod install;
pub(super) mod notice;
pub(super) mod one_off;
pub(super) mod placement;
mod shell_command;
pub(super) mod startup;
pub(super) mod title_bar;
pub(super) mod wallpaper;
pub(super) mod window;

/// 帧的绘制与呈现：渲染器、下一帧的安排和测量。
pub(crate) struct FrameState {
    /// 渲染器；尚未创建时为 `None`。
    pub(in crate::app) renderer: Option<Renderer>,
    /// 动画下一帧的绘制时刻（没有显示链接时按计时器安排）。
    pub(in crate::app) next_frame: Option<Instant>,
    /// 显示器每刷新一次触发一次回调；有动画时据此绘制下一帧（macOS 14 起、Windows）。
    #[cfg(target_os = "macos")]
    pub(in crate::app) display_link: Option<crate::macos::DisplayLink>,
    /// 显示器每刷新一次触发一次回调；有动画时据此绘制下一帧（macOS 14 起、Windows）。
    #[cfg(windows)]
    pub(in crate::app) display_link: Option<crate::windows::DisplayLink>,
    /// 相邻两帧的间隔；动画开始时按窗口所在显示器的刷新率计算。
    pub(in crate::app) frame_interval: Duration,
    /// 测量钩子的状态。
    pub(in crate::app) bench: Bench,
    /// 帧日志（`WINC_FRAME_LOG`）。
    pub(in crate::app) frame_log: crate::frame_log::FrameLog,
    /// 已呈现的帧数，供测量钩子使用。
    pub(in crate::app) presented: usize,
    /// 是否需要重新绘制。
    pub(in crate::app) dirty: bool,
}
