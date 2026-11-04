//! 其他平台的呈现：直接画进 `softbuffer` 的窗口缓冲并呈现。

use std::num::NonZeroU32;
use std::sync::Arc;

use winit::window::Window;

use crate::draw::canvas::Canvas;

/// 软件缓冲呈现器：持有窗口的 `softbuffer` 表面。
pub(crate) struct Presenter {
    /// 窗口对应的 `softbuffer` 表面，缓冲里每个像素为 `0xAARRGGBB`。
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    /// 窗口客户区宽度（物理像素），不小于 1。
    width: u32,
    /// 窗口客户区高度（物理像素），不小于 1。
    height: u32,
}

impl Presenter {
    /// 创建呈现器，呈现目标是 `window`。
    ///
    /// # Errors
    ///
    /// 创建 `softbuffer` 上下文或表面失败时返回错误描述。
    pub(crate) fn new(window: &Arc<Window>) -> Result<Self, String> {
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface =
            softbuffer::Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        let mut presenter = Self {
            surface,
            width: 0,
            height: 0,
        };
        let size = window.inner_size();
        presenter.resize(size.width, size.height, window.scale_factor());
        Ok(presenter)
    }

    /// 更新窗口尺寸（物理像素，小于 1 的按 1 处理）；参数 `_scale` 不使用。
    pub(crate) fn resize(&mut self, width: u32, height: u32, _scale: f64) {
        self.width = width.max(1);
        self.height = height.max(1);
        let to_nonzero = |v: u32| NonZeroU32::new(v).expect("已经保证不为 0");
        let _ = self
            .surface
            .resize(to_nonzero(self.width), to_nonzero(self.height));
    }

    /// 返回当前画面尺寸（物理像素）。
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 把 `paint` 画进 `softbuffer` 的窗口缓冲并呈现；取不到缓冲时跳过本帧。
    ///
    /// 总是返回 `true`：这里不等待缓冲，取不到时在下一次屏幕刷新时重试也取不到。`paint` 直接
    /// 画进窗口缓冲（行距等于画面宽度），不另备一块同样大的像素缓冲再整块拷贝。
    pub(crate) fn frame(&mut self, paint: impl FnOnce(Canvas<'_>)) -> bool {
        let (width, height) = (self.width as usize, self.height as usize);
        let Ok(mut buffer) = self.surface.buffer_mut() else {
            return true;
        };
        // 调整尺寸失败时缓冲还是原来的大小，装不下这一帧，跳过。
        if buffer.len() < width * height {
            return true;
        }
        paint(Canvas {
            px: &mut buffer[..],
            width,
            height,
            stride: width,
        });
        let _ = buffer.present();
        true
    }

    /// 返回应当把空闲的呈现缓冲标成可清除的时刻；`softbuffer` 只有一块不能标成可清除的缓冲，
    /// 总是 `None`。
    pub(crate) fn purgeable_at(&self) -> Option<std::time::Instant> {
        None
    }

    /// 把空闲的呈现缓冲标成可清除；`softbuffer` 的缓冲不能标成可清除，什么也不做。
    pub(crate) fn make_idle_purgeable(&mut self) {}

    /// 设置窗口是否在前台；`softbuffer` 的缓冲不区分前后台，什么也不做。
    pub(crate) fn set_foreground(&mut self, _foreground: bool) {}

    /// 设置窗口是否完全看不见；`softbuffer` 的缓冲不能标成可清除，什么也不做。
    pub(crate) fn set_occluded(&mut self, _occluded: bool) {}
}
