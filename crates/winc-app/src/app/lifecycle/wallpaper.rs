//! 窗口背景图：按配置在后台读取、缩放图片，交给渲染器；图片、不透明度或窗口大小改变时重新生成。
//!
//! 图层按逻辑像素存储（见 [`MAX_SIDE`]），2 倍屏上只占物理像素大小的四分之一；解码后的原图不保留。
//! 没有配置背景图时不占内存，绘制也与原来相同。

use std::path::PathBuf;
use std::sync::mpsc;

use winc_config::Config;
use winc_preview::image::Pixels;
use winc_render::{Color, Renderer, Wallpaper};

use crate::app::{App, UserEvent};

/// 背景图层每边的像素数上限：更大的窗口按该大小生成，显示时放大。
const MAX_SIDE: f32 = 2560.0;

/// 图片文件的大小上限（字节，64 MB）：更大的文件不读取。
const MAX_FILE: u64 = 64 << 20;

/// 一张背景图层由什么生成：图片的路径、不透明度（百分比）和大小（像素）。
#[derive(Debug, Clone, PartialEq)]
struct Source {
    path: PathBuf,
    opacity: u8,
    size: (u32, u32),
}

/// 背景图的状态。
pub(crate) struct WallpaperState {
    /// 渲染器当前的背景图由什么生成；没有背景图时为 `None`。
    shown: Option<Source>,
    /// 正在后台生成的背景图；同一时刻至多一张。
    loading: Option<Source>,
    /// 无法加载的图片：不再重试，换一张图片后清除。
    failed: Option<PathBuf>,
    /// 交给渲染器的底色，配色改变时更新。
    surfaces: [Color; 3],
    sender: mpsc::Sender<(Source, Result<Wallpaper, String>)>,
    receiver: mpsc::Receiver<(Source, Result<Wallpaper, String>)>,
}

impl Default for WallpaperState {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let none = Color::hex(0).alpha(0.0);
        WallpaperState {
            shown: None,
            loading: None,
            failed: None,
            surfaces: [none; 3],
            sender,
            receiver,
        }
    }
}

impl WallpaperState {
    /// 是否还在等后台生成的背景图（测试中等待用）。
    #[cfg(test)]
    pub(crate) fn is_busy(&self) -> bool {
        self.loading.is_some()
    }
}

impl App {
    /// 配置的背景图由什么生成；没有配置时为 `None`。
    fn wallpaper_source(&self) -> Option<Source> {
        let path = self.config.ui_background_image.as_deref()?;
        let path = PathBuf::from(path.trim());
        let opacity = (self.config.ui_background_opacity).unwrap_or(Config::BACKGROUND_OPACITY);
        let window = self.layout.window;
        if window.w < 1.0 || window.h < 1.0 {
            return None;
        }
        // 超过上限的窗口等比缩小。
        let fit = (MAX_SIDE / window.w.max(window.h)).min(1.0);
        Some(Source {
            path,
            opacity: opacity.round() as u8,
            size: (
                (window.w * fit).round() as u32,
                (window.h * fit).round() as u32,
            ),
        })
    }

    /// 让渲染器的背景图与配置、窗口大小和配色一致；每帧绘制前调用。
    ///
    /// 需要重新生成时在后台进行，完成前继续显示原来的背景图（大小不同时由渲染器缩放）。
    pub(in crate::app) fn sync_wallpaper(&mut self, renderer: &mut Renderer) {
        let wanted = self.wallpaper_source();
        let p = self.palette;
        let surfaces = [p.frame_bg, p.sidebar_bg, p.editor_bg];
        let state = &mut self.wallpaper;
        let Some(wanted) = wanted else {
            if state.shown.take().is_some() {
                renderer.set_wallpaper(None, &[]);
            }
            state.failed = None;
            return;
        };
        if state.shown.is_some() && state.surfaces != surfaces {
            renderer.set_wallpaper_surfaces(&surfaces);
        }
        state.surfaces = surfaces;
        let failed = state.failed.as_ref() == Some(&wanted.path);
        if state.shown.as_ref() == Some(&wanted) || state.loading.is_some() || failed {
            return;
        }
        state.failed = None;
        state.loading = Some(wanted.clone());
        let (sender, proxy) = (state.sender.clone(), self.proxy.clone());
        winc_editor::background::spawn(move || {
            let made = make(&wanted);
            if sender.send((wanted, made)).is_ok()
                && let Some(proxy) = &proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 收下后台生成的背景图。
    pub(in crate::app) fn poll_wallpaper(&mut self) {
        while let Ok((source, made)) = self.wallpaper.receiver.try_recv() {
            self.wallpaper.loading = None;
            // 生成期间换了图片或不透明度：结果丢弃，下一帧按新的配置重新生成。
            let current = self.wallpaper_source();
            let same = current.is_some_and(|wanted| {
                (wanted.path == source.path) && wanted.opacity == source.opacity
            });
            if !same {
                self.request_redraw();
                continue;
            }
            match made {
                Ok(wallpaper) => {
                    let surfaces = self.wallpaper.surfaces;
                    if let Some(renderer) = &mut self.renderer {
                        renderer.set_wallpaper(Some(wallpaper), &surfaces);
                        self.wallpaper.shown = Some(source);
                    }
                }
                Err(reason) => {
                    self.wallpaper.failed = Some(source.path);
                    self.notify(winc_i18n::t!("settings-background-failed", reason = reason));
                }
            }
            self.request_redraw();
        }
    }

    /// 设置或取消背景图片，并写回配置。
    pub(in crate::app) fn set_background_image(&mut self, path: Option<PathBuf>) {
        self.config.ui_background_image = path.map(|p| p.to_string_lossy().into_owned());
        self.wallpaper.failed = None;
        self.save_config();
        self.request_redraw();
    }

    /// 测试中等后台生成完背景图并收下。
    #[cfg(test)]
    pub(crate) fn wait_for_wallpaper(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.wallpaper.is_busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll_wallpaper();
        }
    }
}

/// 读取图片并生成背景图层；在后台线程上调用。
fn make(source: &Source) -> Result<Wallpaper, String> {
    let size = std::fs::metadata(&source.path)
        .map_err(|e| e.to_string())?
        .len();
    if size > MAX_FILE {
        return Err(winc_i18n::t!("preview-image-too-large").to_owned());
    }
    let bytes = std::fs::read(&source.path).map_err(|e| e.to_string())?;
    let image = winc_preview::image::decode_bytes(&bytes)?;
    drop(bytes);
    let fitted = cover(&image, source.size);
    drop(image);
    let opacity = f32::from(source.opacity) / 100.0;
    Wallpaper::new(fitted.width, fitted.height, &fitted.rgba, opacity)
        .ok_or_else(|| winc_i18n::t!("preview-image-empty").to_owned())
}

/// 把 `image` 等比缩放并居中裁剪，使它恰好铺满 `size`。
fn cover(image: &Pixels, (width, height): (u32, u32)) -> Pixels {
    let (sw, sh) = (u64::from(image.width), u64::from(image.height));
    let (w, h) = (u64::from(width.max(1)), u64::from(height.max(1)));
    // 先裁出与目标比例相同的区域，再缩放。
    let (cw, ch) = if sw * h > sh * w {
        ((sh * w / h).max(1), sh)
    } else {
        (sw, (sw * h / w).max(1))
    };
    let (x0, y0) = (((sw - cw) / 2) as usize, ((sh - ch) / 2) as usize);
    let (cw, ch, stride) = (cw as usize, ch as usize, sw as usize * 4);
    let mut rgba = Vec::with_capacity(cw * ch * 4);
    for row in image.rgba.chunks_exact(stride).skip(y0).take(ch) {
        rgba.extend_from_slice(&row[x0 * 4..(x0 + cw) * 4]);
    }
    let cropped = Pixels {
        width: cw as u32,
        height: ch as u32,
        rgba,
    };
    winc_preview::image::resize(&cropped, width, height)
}
