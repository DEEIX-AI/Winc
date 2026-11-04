//! 呈现：把画好的一帧交给窗口。
//!
//! macOS 画进 `IOSurface`（`macos` 模块），其他平台画进软件缓冲（`soft` 模块）。

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod soft;

#[cfg(target_os = "macos")]
pub(crate) use macos::Presenter;
#[cfg(not(target_os = "macos"))]
pub(crate) use soft::Presenter;
