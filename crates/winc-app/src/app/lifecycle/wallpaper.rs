//! 窗口背景图：按配置在后台生成图层，交给渲染器。
//!
//! 图层保持图片自身的宽高比，由渲染器按窗口等比缩放、居中裁剪，窗口大小改变时不必重新生成；
//! 只有图层相对窗口过小或过大（见 [`GROW_AT`]、[`SHRINK_AT`]）时才重新生成。生成好的图层存在
//! 磁盘上（见 `cache`），下次启动、调整不透明度时直接读取，不再解码原图。同一张图层在各窗口
//! 之间共用。没有配置背景图时不占内存，绘制也与原来相同。

mod cache;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, mpsc};

use winc_config::Config;
use winc_render::{Color, Renderer, Wallpaper};

use crate::app::{App, UserEvent};

/// 图层长边的像素数上限：图片在底色之下只透出一部分，更高的分辨率看不出差别。
const MAX_SIDE: f32 = 1920.0;

/// 图片文件的大小上限（字节，64 MB）：更大的文件不读取。
const MAX_FILE: u64 = 64 << 20;

/// 窗口所需的图层比现有的大到该倍数以上时重新生成：再放大显示会明显发糊。
const GROW_AT: f32 = 1.25;

/// 窗口所需的图层比现有的小到该倍数以下时重新生成，释放多占的内存。
const SHRINK_AT: f32 = 0.6;

/// 一张背景图层由什么生成：图片的路径和不透明度（百分比）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    path: PathBuf,
    opacity: u8,
}

/// 生成好的背景图层。
pub(crate) struct Layer {
    source: Source,
    wallpaper: Arc<Wallpaper>,
    /// 原图解码后的大小（像素）：据此判断窗口变大后能否生成更清晰的图层。
    full: (u32, u32),
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size = self.wallpaper.size();
        write!(f, "Layer({:?}, {size:?})", self.source)
    }
}

/// 后台生成的结果：图层和原图解码后的大小，或失败的原因。
type Made = Result<(Wallpaper, (u32, u32)), String>;

/// 背景图的状态。
pub(crate) struct WallpaperState {
    /// 渲染器当前的图层；没有背景图时为 `None`。
    shown: Option<Rc<Layer>>,
    /// 正在后台生成的背景图；同一时刻至多一张。
    loading: Option<Source>,
    /// 无法加载的图片：不再重试，换一张图片后清除。
    failed: Option<PathBuf>,
    /// 交给渲染器的底色，配色改变时更新。
    surfaces: [Color; 3],
    sender: mpsc::Sender<(Source, Made)>,
    receiver: mpsc::Receiver<(Source, Made)>,
    /// 已从 `receiver` 取出、等下一帧交给渲染器的结果。
    arrived: Vec<(Source, Made)>,
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
            arrived: Vec::new(),
        }
    }
}

impl WallpaperState {
    /// 是否还在等后台生成的背景图（测试中等待用）。
    #[cfg(test)]
    pub(crate) fn is_busy(&self) -> bool {
        self.loading.is_some() && self.arrived.is_empty()
    }
}

/// 返回宽 `window.0`、高 `window.1`（逻辑像素）的窗口所需的图层大小：`full` 大小的图片等比
/// 缩小到刚好铺满窗口，长边不超过 [`MAX_SIDE`]；不放大，图片较小时即图片自身的大小。
fn target(full: (u32, u32), window: (f32, f32)) -> (u32, u32) {
    let (w, h) = (full.0.max(1) as f32, full.1.max(1) as f32);
    let cover = (window.0 / w).max(window.1 / h);
    let scale = cover.min(MAX_SIDE / w.max(h)).min(1.0);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

/// `layer` 大小的图层对 `window` 是否已不合适：所需的大小与它相差过多（见 [`GROW_AT`]、
/// [`SHRINK_AT`]）。`full` 是原图解码后的大小。
fn stale(layer: (u32, u32), full: (u32, u32), window: (f32, f32)) -> bool {
    let ratio = target(full, window).0 as f32 / layer.0.max(1) as f32;
    !(SHRINK_AT..=GROW_AT).contains(&ratio)
}

impl App {
    /// 配置的背景图由什么生成；没有配置，或不透明度为 0（图片不可见）时为 `None`。
    ///
    /// 测量时不显示背景图，测量结果不随用户的配置变化。
    fn wallpaper_source(&self) -> Option<Source> {
        let path = (self.config.ui_background_image.as_deref()).filter(|_| !self.bench.silent())?;
        let opacity = (self.config.ui_background_opacity).unwrap_or(Config::BACKGROUND_OPACITY);
        (opacity >= 0.5).then(|| Source {
            path: PathBuf::from(path.trim()),
            opacity: opacity.round() as u8,
        })
    }

    /// 图层缓存文件的位置：与状态文件在同一个目录；没有状态文件（测量、测试）时不缓存。
    fn wallpaper_cache(&self) -> Option<PathBuf> {
        Some(
            self.state_path
                .as_deref()?
                .parent()?
                .join("wallpaper.cache"),
        )
    }

    /// 为宽高为 `window`（逻辑像素）的窗口准备背景图层：现有的或其他窗口的图层合用时直接
    /// 采用，否则在后台生成。
    ///
    /// 窗口创建之前即可调用（按预计的窗口大小），使图层在第一帧之前就绪。
    pub(in crate::app) fn request_wallpaper(&mut self, window: (f32, f32)) {
        let Some(wanted) = self.wallpaper_source() else {
            return;
        };
        if window.0 < 1.0 || window.1 < 1.0 {
            return;
        }
        let fits = |layer: &Layer| {
            layer.source == wanted && !stale(layer.wallpaper.size(), layer.full, window)
        };
        if self.wallpaper.shown.as_deref().is_some_and(fits) {
            return;
        }
        // 其他窗口已有合用的图层：共用，不再生成。
        let shared = self.shared.wallpaper.borrow().upgrade();
        if let Some(layer) = shared.filter(|layer| fits(layer)) {
            self.wallpaper.shown = Some(layer);
            self.request_redraw();
            return;
        }
        let state = &mut self.wallpaper;
        let failed = state.failed.as_ref() == Some(&wanted.path);
        if state.loading.is_some() || failed {
            return;
        }
        state.failed = None;
        state.loading = Some(wanted.clone());
        let (sender, proxy) = (state.sender.clone(), self.proxy.clone());
        let cache = self.wallpaper_cache();
        winc_editor::background::spawn(move || {
            let made = make(&wanted, window, cache.as_deref());
            if sender.send((wanted, made)).is_ok()
                && let Some(proxy) = &proxy
            {
                let _ = proxy.send_event(UserEvent::ImageReady);
            }
        });
    }

    /// 让渲染器的背景图与配置、窗口大小和配色一致；每帧绘制前调用。
    ///
    /// 需要重新生成时在后台进行，完成前继续显示原来的图层。
    pub(in crate::app) fn sync_wallpaper(&mut self, renderer: &mut Renderer) {
        self.accept_wallpapers();
        let p = self.palette;
        let surfaces = [p.frame_bg, p.sidebar_bg, p.editor_bg];
        if self.wallpaper_source().is_none() {
            self.wallpaper.shown = None;
            self.wallpaper.failed = None;
        } else {
            let window = self.layout.window;
            self.request_wallpaper((window.w, window.h));
        }
        // 渲染器的图层与状态不一致（刚生成好、从其他窗口采用，或已取消）时更换。
        let state = &mut self.wallpaper;
        let layer = state.shown.as_ref().map(|layer| &layer.wallpaper);
        let same = match (layer, renderer.wallpaper()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            renderer.set_wallpaper(layer.cloned(), &surfaces);
        } else if layer.is_some() && state.surfaces != surfaces {
            renderer.set_wallpaper_surfaces(&surfaces);
        }
        state.surfaces = surfaces;
    }

    /// 收下后台生成的图层。
    fn accept_wallpapers(&mut self) {
        let state = &mut self.wallpaper;
        state.arrived.extend(state.receiver.try_iter());
        for (source, made) in std::mem::take(&mut self.wallpaper.arrived) {
            self.wallpaper.loading = None;
            // 生成期间换了图片或不透明度：结果丢弃，随后按新的配置重新生成。
            if self.wallpaper_source().as_ref() != Some(&source) {
                continue;
            }
            match made {
                Ok((wallpaper, full)) => {
                    let layer = Rc::new(Layer {
                        source,
                        wallpaper: Arc::new(wallpaper),
                        full,
                    });
                    *self.shared.wallpaper.borrow_mut() = Rc::downgrade(&layer);
                    self.wallpaper.shown = Some(layer);
                }
                Err(reason) => {
                    self.wallpaper.failed = Some(source.path);
                    self.notify(winc_i18n::t!("settings-background-failed", reason = reason));
                }
            }
        }
    }

    /// 后台有图层生成完成：重画一帧，由 [`Self::sync_wallpaper`] 收下。
    pub(in crate::app) fn poll_wallpaper(&mut self) {
        if self.wallpaper.loading.is_some() {
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

    /// 测试中等后台生成完背景图。
    #[cfg(test)]
    pub(crate) fn wait_for_wallpaper(&mut self) {
        let wait = std::time::Duration::from_secs(10);
        if self.wallpaper.is_busy()
            && let Ok(done) = self.wallpaper.receiver.recv_timeout(wait)
        {
            self.wallpaper.arrived.push(done);
        }
    }
}

/// 生成 `source` 的图层，供宽高为 `window`（逻辑像素）的窗口使用；在后台线程上调用。
///
/// 缓存 `cache` 中有同一张图片、大小合用的图层时直接读取，否则解码原图并写入缓存。
fn make(source: &Source, window: (f32, f32), cache: Option<&Path>) -> Made {
    let metadata = std::fs::metadata(&source.path).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_FILE {
        return Err(winc_i18n::t!("preview-image-too-large").to_owned());
    }
    let opacity = f32::from(source.opacity) / 100.0;
    let empty = || winc_i18n::t!("preview-image-empty").to_owned();
    let stamp = cache::Stamp::of(&source.path, &metadata);
    let cached = cache.and_then(|file| cache::read(file, &stamp));
    if let Some(layer) = cached.filter(|layer| !stale(layer.size, layer.full, window)) {
        let (w, h) = layer.size;
        let wallpaper = Wallpaper::new(w, h, &layer.rgba, opacity).ok_or_else(empty)?;
        return Ok((wallpaper, layer.full));
    }
    let bytes = std::fs::read(&source.path).map_err(|e| e.to_string())?;
    let image = winc_preview::image::decode_bytes(&bytes)?;
    drop(bytes);
    let full = (image.width, image.height);
    let size = target(full, window);
    let rgba = if size == full {
        image.rgba
    } else {
        winc_preview::image::resize(&image, size.0, size.1).rgba
    };
    let wallpaper = Wallpaper::new(size.0, size.1, &rgba, opacity).ok_or_else(empty)?;
    if let Some(file) = cache {
        cache::write(file, &stamp, &cache::Cached { size, full, rgba });
    }
    Ok((wallpaper, full))
}

#[cfg(test)]
mod tests;
