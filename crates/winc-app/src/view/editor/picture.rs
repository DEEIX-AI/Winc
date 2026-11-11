//! 图片和 SVG 的预览、分栏预览。

use winc_i18n::t;

use super::*;
use crate::app::preview::images::PictureCache;

/// SVG 缓存长边的最大尺寸（物理像素）。
///
/// 最多保留 64 MiB 的 RGBA 像素；继续放大时按显示矩形取样，不分配整张放大的图片。
/// SVG 的组和滤镜还会使用临时图层，此值只限制最终缓存的尺寸。
const VECTOR_CACHE_MAX: u32 = 4096;

/// 返回渲染缓存的尺寸：位图不超过保留的源像素，SVG 按比例限制最长边。
fn cache_size(source: &Source, (width, height): (u32, u32)) -> (u32, u32) {
    match source {
        Source::Raster(pixels) => (width.min(pixels.width), height.min(pixels.height)),
        Source::Vector(_) => {
            let ratio = (VECTOR_CACHE_MAX as f32 / width.max(height) as f32).min(1.0);
            (
                (width as f32 * ratio).round().max(1.0) as u32,
                (height as f32 * ratio).round().max(1.0) as u32,
            )
        }
    }
}

/// 把棋盘范围裁到可见区域，起点按两个格子对齐，保留原图上的颜色相位。
///
/// 淡入淡出时棋盘按格混合；只保留可见格子，避免大倍率图片产生数百万个不可见格子。
fn checker_bounds(rect: Rect, clip: Rect, cell: f32) -> Rect {
    let visible = rect.intersect(clip);
    if visible.w <= 0.0 || visible.h <= 0.0 {
        return Rect::default();
    }
    let period = cell * 2.0;
    let x = rect.x + ((visible.x - rect.x) / period).floor() * period;
    let y = rect.y + ((visible.y - rect.y) / period).floor() * period;
    Rect::new(x, y, visible.right() - x, visible.bottom() - y)
}

/// 绘制分栏预览：编辑区右边那一半，左边一道分隔线，中间是跟着文字重绘的图（`App::split`）。
///
/// 写到一半解析不了时仍然绘制上一张图，底下一行写出错在哪里；一张都还没绘制出来时只写原因。
pub(in crate::view) fn draw_split(app: &mut App, frame: &mut Frame<'_>) {
    let area = app.layout.split;
    let Some(doc) = app.editor.active().filter(|_| area.w > 0.0) else {
        return;
    };
    let (id, kind) = (doc.id, doc.preview_kind());
    let p = app.palette;
    frame.rect(area, p.editor_bg);
    // 悬停、拖动分隔线时用强调色，表示可以左右拖动。
    let active = app.split_divider_active();
    let line = if active { p.accent } else { p.border };
    frame.rect(Rect::new(area.x - 1.0, area.y, 1.0, area.h), line);
    if kind == Some(winc_editor::PreviewKind::Markdown) {
        crate::view::preview::markdown::draw(app, frame, area);
        return;
    }
    if kind == Some(winc_editor::PreviewKind::Table) {
        // 尚未读好时留空，读好后会重新绘制。
        if let Some(index) = app.editor.index_of(id) {
            crate::view::preview::table::draw(app, frame, index, area);
        }
        return;
    }
    if app.split.doc != Some(id) {
        return;
    }
    if let Some(picture) = app.split.picture.clone() {
        draw_picture(app, frame, id, &picture, area);
    }
    let Some(reason) = app.split.error.clone() else {
        return;
    };
    let message = t!("editor-preview-failed", reason = reason);
    let line = Rect::new(area.x + 16.0, area.bottom() - 32.0, area.w - 32.0, 20.0);
    let text = ellipsize(frame, &message, app.fonts.ui_small, line.w);
    ui_text_small(frame, app, line.x, line, &text, p.error, area);
}

/// 按当前视图的缩放和平移绘制图片；初始位置和适应窗口的尺寸由 `PictureView` 计算。
///
/// 有透明区域的图片底下铺棋盘格；高倍率放大只改变显示矩形，缓存维持有界尺寸。
pub(super) fn draw_picture(
    app: &mut App,
    frame: &mut Frame<'_>,
    id: winc_editor::DocumentId,
    picture: &Arc<winc_preview::image::Picture>,
    editor: Rect,
) {
    // 棋盘格的边长（逻辑像素）。
    const CHECKER: f32 = 8.0;

    let p = app.palette;
    app.sync_picture_view(id, picture, editor);
    let rect = app.picture_view.rect();
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let scale = frame.scale();
    // 缓存尺寸不受平移和边界取整影响，避免拖动时反复重绘 SVG。
    let (pw, ph) = (
        (rect.w * scale).round().max(1.0) as u32,
        (rect.h * scale).round().max(1.0) as u32,
    );
    let size = cache_size(&picture.source, (pw, ph));
    // 只在绘制时对齐物理像素，交互状态保留小数，缩放锚点不会随取整漂移。
    let (x, y) = ((rect.x * scale).round(), (rect.y * scale).round());
    let (dw, dh) = (
        ((rect.right() * scale).round() - x).max(1.0) as u32,
        ((rect.bottom() * scale).round() - y).max(1.0) as u32,
    );
    let rect = Rect::new(x / scale, y / scale, dw as f32 / scale, dh as f32 / scale);

    if picture.transparent {
        // 底色上隔一格叠一层悬停色；底色不透明，叠好的颜色也不透明，整片一遍填完。
        let (base, over) = (p.selected_inactive, p.hover);
        let mix = |b: f32, o: f32| b + (o - b) * over.a;
        let tinted = Color {
            r: mix(base.r, over.r),
            g: mix(base.g, over.g),
            b: mix(base.b, over.b),
            a: base.a,
        };
        let cell = (CHECKER * scale).round().max(1.0) / scale;
        frame.checker(
            checker_bounds(rect, editor, cell),
            cell,
            (tinted, base),
            editor,
        );
    }

    // 缓存中是否是这张图；是这张图时，记录其当前尺寸。
    let same = app
        .picture_cache
        .as_ref()
        .filter(|c| c.doc == id && Arc::ptr_eq(&c.picture, picture))
        .map(|c| (c.pixels.width, c.pixels.height));
    if same != Some(size) {
        match &picture.source {
            // 缩小使用按透明度加权的平均；放大复用源像素，不生成大尺寸位图。
            Source::Raster(pixels) => {
                app.picture_cache = Some(PictureCache {
                    doc: id,
                    picture: Arc::clone(picture),
                    pixels: if size == (pixels.width, pixels.height) {
                        pixels.clone()
                    } else {
                        picture.render(size.0, size.1)
                    },
                });
            }
            // SVG 带滤镜时光栅化一次可能耗时较长：交给后台线程。等待期间直接按当前矩形
            // 绘制旧缓存，不为每次缩放分配临时像素。
            Source::Vector(_) => {
                app.render_picture(id, picture, size);
                if same.is_none() {
                    app.picture_cache = None;
                }
            }
        }
    }
    if let Some(cache) = &app.picture_cache {
        let px = &cache.pixels;
        if (px.width, px.height) == (dw, dh) {
            frame.pixels((rect.x, rect.y), px.width, px.height, &px.rgba, editor);
        } else {
            frame.pixels_scaled(rect, px.width, px.height, &px.rgba, editor);
        }
    }
}

#[cfg(test)]
mod tests;
