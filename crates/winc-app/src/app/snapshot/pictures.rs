//! 图片预览的鼠标缩放、拖动和裁剪回归测试。

use super::*;
use winit::event::TouchPhase;

fn picture_app(name: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("image.jpg");
    std::fs::write(
        &path,
        include_bytes!("../../../../winc-preview/testdata/red-8x8.jpg"),
    )
    .unwrap();
    let mut app = app_with(&path, false);
    app.wait_for_images();
    paint(&mut app);
    (app, dir)
}

fn wheel(app: &mut App, y: f32) {
    app.on_wheel_event(MouseScrollDelta::LineDelta(0.0, y), TouchPhase::Moved);
    paint(app);
}

#[test]
fn image_wheel_changes_the_rendered_picture_without_changing_text_size() {
    let (mut app, dir) = picture_app("picture-wheel");
    let area = app.layout.editor;
    app.on_mouse_move(area.x + area.w / 2.0, area.y + area.h / 2.0);
    let before = app
        .renderer
        .as_ref()
        .unwrap()
        .image()
        .unwrap()
        .pixels
        .to_vec();
    let font = app.fonts.line_height;
    wheel(&mut app, 1.0);
    let after = app
        .renderer
        .as_ref()
        .unwrap()
        .image()
        .unwrap()
        .pixels
        .to_vec();
    assert!(before != after, "滚轮应改变实际绘制的图片");
    assert_eq!(app.fonts.line_height, font);
    std::fs::remove_dir_all(dir).unwrap();
}

fn anchor(app: &App) -> (f32, f32) {
    let rect = app.picture_view.rect();
    (
        (app.mouse.0 - rect.x) / rect.w,
        (app.mouse.1 - rect.y) / rect.h,
    )
}

fn near(a: (f32, f32), b: (f32, f32)) {
    assert!(
        (a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001,
        "{a:?} != {b:?}"
    );
}

#[test]
fn image_zoom_keeps_the_pointer_anchor_at_limits_and_with_pixel_wheels() {
    let (mut app, dir) = picture_app("picture-anchor");
    let rect = app.picture_view.rect();
    app.on_mouse_move(rect.x + rect.w * 0.3, rect.y + rect.h * 0.7);
    let at = anchor(&app);
    let font = app.fonts.line_height;
    app.modifiers = command_modifiers();
    wheel(&mut app, 1.0);
    near(at, anchor(&app));
    assert_eq!(app.fonts.line_height, font);
    app.on_wheel_event(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 48.0 * f64::from(SCALE))),
        TouchPhase::Started,
    );
    paint(&mut app);
    near(at, anchor(&app));
    assert!((app.picture_view.zoom - 1.44).abs() < 0.001);
    wheel(&mut app, 100_000.0);
    assert_eq!(app.picture_view.zoom, 64.0);
    near(at, anchor(&app));
    wheel(&mut app, 100_000.0);
    near(at, anchor(&app));
    wheel(&mut app, -100_000.0);
    assert_eq!(app.picture_view.zoom, 0.1);
    near(at, anchor(&app));
    let pan = app.picture_view.pan;
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0] {
        wheel(&mut app, value);
        assert_eq!(app.picture_view.pan, pan);
        assert_eq!(app.picture_view.zoom, 0.1);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn image_drag_tracks_the_pointer_and_double_click_restores_fit() {
    use winit::window::CursorIcon;
    let (mut app, dir) = picture_app("picture-drag");
    let rect = app.picture_view.rect();
    let start = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    app.on_mouse_move(start.0, start.1);
    assert_eq!(app.cursor_icon, CursorIcon::Grab);
    wheel(&mut app, 5.0);
    app.on_mouse_down(MouseButton::Left);
    assert_eq!(app.cursor_icon, CursorIcon::Grabbing);
    app.on_mouse_move(start.0 + 70.0, start.1 - 30.0);
    near(app.picture_view.pan, (70.0, -30.0));
    // 拖动中滚轮缩放不会在下一次移动时跳回旧偏移。
    wheel(&mut app, 1.0);
    let pan = app.picture_view.pan;
    app.on_mouse_move(app.mouse.0 + 10.0, app.mouse.1 + 20.0);
    near(app.picture_view.pan, (pan.0 + 10.0, pan.1 + 20.0));
    app.on_mouse_up(MouseButton::Left);
    let pan = app.picture_view.pan;
    app.on_mouse_move(start.0, start.1);
    near(app.picture_view.pan, pan);
    assert_eq!(app.cursor_icon, CursorIcon::Grab);
    app.last_click = None;
    click(&mut app, start);
    click(&mut app, start);
    paint(&mut app);
    assert_eq!(app.picture_view.zoom, 1.0);
    near(app.picture_view.pan, (0.0, 0.0));
    near(
        (app.picture_view.rect().w, app.picture_view.rect().h),
        (8.0, 8.0),
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn image_zoom_is_blocked_by_overlays_and_does_not_leak_into_other_tabs() {
    let (mut app, dir) = picture_app("picture-overlays");
    let area = app.layout.editor;
    let point = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    app.on_mouse_move(point.0, point.1);
    app.settings.open = true;
    wheel(&mut app, 1.0);
    assert_eq!(app.picture_view.zoom, 1.0);
    app.settings.open = false;
    paint(&mut app);
    app.picker.open = true;
    wheel(&mut app, 1.0);
    assert_eq!(app.picture_view.zoom, 1.0);
    app.picker.open = false;
    paint(&mut app);
    wheel(&mut app, 2.0);
    assert!(app.picture_view.zoom > 1.0);
    app.on_mouse_down(MouseButton::Left);
    app.new_file();
    app.on_mouse_move(point.0 + 50.0, point.1);
    assert_eq!(app.drag, Drag::None);
    paint(&mut app);
    assert_eq!(app.picture_view.doc, None);
    app.activate_tab(0);
    app.wait_for_images();
    paint(&mut app);
    assert_eq!(app.picture_view.zoom, 1.0);
    near(app.picture_view.pan, (0.0, 0.0));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn image_enlargement_reuses_source_pixels_and_stays_inside_the_preview() {
    let (mut app, dir) = picture_app("picture-cache");
    let rect = app.picture_view.rect();
    app.on_mouse_move(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    wheel(&mut app, 20.0);
    let cache = app.picture_cache.as_ref().unwrap();
    assert_eq!((cache.width(), cache.height()), (8, 8));
    let allocation = cache.pixels.rgba.as_ptr();
    wheel(&mut app, 20.0);
    assert_eq!(
        app.picture_cache.as_ref().unwrap().pixels.rgba.as_ptr(),
        allocation
    );
    let before = app
        .renderer
        .as_ref()
        .unwrap()
        .image()
        .unwrap()
        .pixels
        .to_vec();
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(0.0, 0.0);
    paint(&mut app);
    let after = &app.renderer.as_ref().unwrap().image().unwrap().pixels;
    let image_width = (SIZE.0 * SCALE) as usize;
    let sidebar_width = (app.layout.sidebar.w * SCALE) as usize;
    let top = (TOP_BAR_END * SCALE) as usize;
    let bottom = ((SIZE.1 - 50.0) * SCALE) as usize;
    for row in top..bottom {
        let range = row * image_width..row * image_width + sidebar_width;
        assert_eq!(&before[range.clone()], &after[range], "图片不得绘制到侧栏");
    }
    app.on_mouse_up(MouseButton::Left);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replacing_an_image_cancels_the_previous_drag() {
    let (mut app, dir) = picture_app("picture-reload-drag");
    let area = app.layout.editor;
    let start = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    app.on_mouse_move(start.0, start.1);
    wheel(&mut app, 4.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(start.0 + 40.0, start.1 + 20.0);
    let replacement = app
        .editor
        .active()
        .unwrap()
        .picture()
        .unwrap()
        .as_ref()
        .clone();
    app.editor.active_mut().unwrap().picture = Some(winc_editor::PictureState::Ready(
        std::sync::Arc::new(replacement),
    ));
    paint(&mut app);
    app.on_mouse_move(start.0 + 80.0, start.1 + 50.0);
    assert_eq!(app.drag, Drag::None);
    assert_eq!(app.picture_view.zoom, 1.0);
    near(app.picture_view.pan, (0.0, 0.0));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn opening_a_picker_cancels_picture_dragging() {
    let (mut app, dir) = picture_app("picture-picker-drag");
    let area = app.layout.editor;
    app.on_mouse_move(area.x + area.w / 2.0, area.y + area.h / 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.open_picker();
    let pan = app.picture_view.pan;
    app.on_mouse_move(app.mouse.0 + 50.0, app.mouse.1 + 20.0);
    near(app.picture_view.pan, pan);
    assert_eq!(app.drag, Drag::None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn returning_to_an_image_does_not_resume_an_old_drag() {
    let (mut app, dir) = picture_app("picture-return-drag");
    let area = app.layout.editor;
    let start = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    app.on_mouse_move(start.0, start.1);
    app.on_mouse_down(MouseButton::Left);
    app.new_file();
    paint(&mut app);
    app.activate_tab(0);
    paint(&mut app);
    app.on_mouse_move(start.0 + 50.0, start.1 + 20.0);
    near(app.picture_view.pan, (0.0, 0.0));
    assert_eq!(app.drag, Drag::None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn svg_split_zoom_leaves_source_scroll_and_font_unchanged() {
    use winc_editor::PreviewMode;
    let dir = sample_folder("picture-svg-split");
    let path = dir.join("sample.svg");
    std::fs::write(
        &path,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 800">
<rect width="1200" height="800" fill="#e8f0fc"/>
<rect x="40" y="40" width="500" height="320" fill="#367df2"/>
<circle cx="870" cy="390" r="220" fill="#ec704a"/>
<path d="M80 700L1120 100M80 100L1120 700" stroke="#19334d" stroke-width="12"/>
</svg>"##,
    )
    .unwrap();
    let mut app = app_with(&path, false);
    app.set_preview_mode(PreviewMode::Split);
    app.wait_for_split();
    app.wait_for_images();
    paint(&mut app);
    let area = app.layout.split;
    app.on_mouse_move(area.x + area.w * 0.35, area.y + area.h * 0.65);
    let at = anchor(&app);
    let font = app.fonts.line_height;
    let scroll = app.editor.active().unwrap().scroll_line;
    shoot(&mut app, "picture-svg-before");
    wheel(&mut app, 6.0);
    app.wait_for_images();
    paint(&mut app);
    near(at, anchor(&app));
    assert_eq!(app.fonts.line_height, font);
    assert_eq!(app.editor.active().unwrap().scroll_line, scroll);
    shoot(&mut app, "picture-svg-zoomed");
    wheel(&mut app, 100_000.0);
    app.wait_for_images();
    paint(&mut app);
    let cache = app.picture_cache.as_ref().unwrap();
    assert!(cache.width().max(cache.height()) <= 4096);
    app.set_preview_mode(PreviewMode::Preview);
    app.wait_for_images();
    paint(&mut app);
    let area = app.layout.editor;
    let point = (area.x + area.w * 0.4, area.y + area.h * 0.6);
    app.on_mouse_move(point.0, point.1);
    let at = anchor(&app);
    wheel(&mut app, -3.0);
    near(at, anchor(&app));
    app.on_mouse_down(MouseButton::Left);
    let pan = app.picture_view.pan;
    app.on_mouse_move(point.0 + 30.0, point.1 - 20.0);
    near(app.picture_view.pan, (pan.0 + 30.0, pan.1 - 20.0));
    app.on_mouse_up(MouseButton::Left);
    app.set_preview_mode(PreviewMode::Source);
    paint(&mut app);
    assert_eq!(app.picture_view.doc, None);
    assert!(app.editor.active().unwrap().shows_text());
    std::fs::remove_dir_all(dir).unwrap();
}
