//! 触摸屏：点按、长按菜单、单指滚动，以及窗口、滚动条和分隔条的拖动。
//!
//! 点按直到抬手才发出，滑动列表不会先打开文件或执行按钮。多指接触取消当前手势，等所有手指
//! 离开后再识别下一次操作；取消和失焦也不会补发点击。

use std::time::{Duration, Instant};

use winit::dpi::PhysicalPosition;
use winit::event::{DeviceId, MouseButton, MouseScrollDelta, Touch, TouchPhase};
use winit::window::ResizeDirection;

use super::*;

#[cfg(test)]
mod tests;

type Point = (f32, f32);

/// 标题栏或窗口边缘的触摸操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum WindowTouch {
    /// 拖动标题栏移动窗口。
    Move,
    /// 拖动边缘或角落改变窗口大小。
    Resize(ResizeDirection),
}

/// 手指移动超过此距离（逻辑像素）后开始滚动，容许点按和长按时的轻微抖动。
const SLOP: f32 = 8.0;
/// 静止按住多久后打开菜单。
const LONG_PRESS: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Pending,
    Scroll,
    Drag,
    Held,
}

struct Gesture {
    origin: Point,
    last: Point,
    since: Instant,
    mode: Mode,
}

/// 每个窗口各自保存的触摸手势；接触点按设备和编号共同识别。
#[derive(Default)]
pub(in crate::app) struct TouchState {
    contacts: Vec<(DeviceId, u64)>,
    gesture: Option<Gesture>,
    #[cfg(windows)]
    pub(in crate::app) window_input: Option<crate::windows::WindowInput>,
}

#[derive(Debug, PartialEq)]
enum Action {
    Tap(Point),
    Context(Point),
    Scroll { origin: Point, delta: Point },
    Press(Point),
    Move(Point),
    Release(Point),
    CancelDrag,
}

impl TouchState {
    /// 是否有手指仍接触屏幕，包括因多指操作而取消的手势。
    pub(in crate::app) fn active(&self) -> bool {
        !self.contacts.is_empty()
    }

    fn update(
        &mut self,
        touch: &Touch,
        point: Point,
        now: Instant,
        direct: bool,
    ) -> Option<Action> {
        let key = (touch.device_id, touch.id);
        if touch.phase == TouchPhase::Started {
            if self.contacts.contains(&key) {
                return None;
            }
            self.contacts.push(key);
            if self.contacts.len() > 1 {
                return self.cancel_gesture();
            }
            self.gesture = Some(Gesture {
                origin: point,
                last: point,
                since: now,
                mode: if direct { Mode::Drag } else { Mode::Pending },
            });
            return direct.then_some(Action::Press(point));
        }

        if !self.contacts.contains(&key) {
            return None;
        }
        if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.contacts.retain(|&contact| contact != key);
        }
        if touch.phase == TouchPhase::Cancelled {
            return self.cancel_gesture();
        }

        let gesture = self.gesture.as_mut()?;
        let ended = touch.phase == TouchPhase::Ended;
        let mut delta = (point.0 - gesture.last.0, point.1 - gesture.last.1);
        if gesture.mode == Mode::Pending {
            let from_start = (point.0 - gesture.origin.0, point.1 - gesture.origin.1);
            if from_start.0.hypot(from_start.1) > SLOP {
                gesture.mode = Mode::Scroll;
                // 越过阈值时包括最初几像素，内容与手指的位移保持一致。
                delta = from_start;
            }
        }
        gesture.last = point;
        let action = match gesture.mode {
            Mode::Pending if ended => Some(if now.duration_since(gesture.since) >= LONG_PRESS {
                Action::Context(gesture.origin)
            } else {
                Action::Tap(gesture.origin)
            }),
            Mode::Scroll if delta != (0.0, 0.0) => Some(Action::Scroll {
                origin: gesture.origin,
                delta,
            }),
            Mode::Drag => Some(if ended {
                Action::Release(point)
            } else {
                Action::Move(point)
            }),
            _ => None,
        };
        if ended {
            self.cancel_gesture();
        }
        action
    }

    fn cancel_gesture(&mut self) -> Option<Action> {
        self.gesture
            .take()
            .and_then(|g| (g.mode == Mode::Drag).then_some(Action::CancelDrag))
    }

    fn deadline(&self) -> Option<Instant> {
        self.gesture
            .as_ref()
            .filter(|g| g.mode == Mode::Pending)
            .map(|g| g.since + LONG_PRESS)
    }

    fn hold(&mut self, now: Instant) -> Option<Action> {
        if now < self.deadline()? {
            return None;
        }
        let gesture = self.gesture.as_mut()?;
        gesture.mode = Mode::Held;
        Some(Action::Context(gesture.origin))
    }
}

impl App {
    /// 按渲染器的缩放换算触摸坐标，再把手势交给现有的点击和滚动处理。
    pub(in crate::app) fn on_touch(&mut self, touch: Touch) {
        let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
        let point = (
            touch.location.x as f32 / scale,
            touch.location.y as f32 / scale,
        );
        if touch.phase == TouchPhase::Started && !self.touch.active() {
            self.drop_drags();
            self.scroll_glide = None;
            self.wheel_glide = None;
            self.dismiss_hint();
            self.cursor_inside = true;
            self.on_mouse_move(point.0, point.1);
            if let Some(operation) = self.touch_window_at(point.0, point.1) {
                #[cfg(windows)]
                {
                    self.touch.update(&touch, point, Instant::now(), false);
                    self.touch.gesture.as_mut().unwrap().mode = Mode::Held;
                }
                self.start_touch_window(point, operation);
                return;
            }
        }
        if touch.phase == TouchPhase::Cancelled
            || (touch.phase == TouchPhase::Started && self.touch.active())
        {
            self.last_click = None;
        }
        let direct = touch.phase == TouchPhase::Started
            && !self.touch.active()
            && self.touch_drag_at(point.0, point.1);
        let action = self.touch.update(&touch, point, Instant::now(), direct);
        if let Some(action) = action {
            self.apply_touch(action);
        }
        if !self.touch.active() {
            self.cursor_inside = false;
            self.set_hover(Hover::None);
        }
    }

    fn start_touch_window(&mut self, (x, y): Point, operation: WindowTouch) {
        if operation == WindowTouch::Move && self.title_bar_pressed(x, y) {
            self.title_bar_double_click();
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        let _ = match operation {
            WindowTouch::Move => window.drag_window(),
            WindowTouch::Resize(direction) => window.drag_resize_window(direction),
        };
    }

    /// 静止长按也需要唤醒事件循环，不依赖新的移动事件。
    pub(in crate::app) fn touch_tick(&mut self, now: Instant) -> Option<Instant> {
        if let Some(action) = self.touch.hold(now) {
            self.apply_touch(action);
        }
        self.touch.deadline()
    }

    /// 窗口失焦或坐标系变化时中断触摸，之后迟到的移动和抬手事件不产生操作。
    pub(in crate::app) fn cancel_touch(&mut self) {
        let active = self.touch.active();
        if let Some(action) = self.touch.cancel_gesture() {
            self.apply_touch(action);
        }
        self.touch.contacts.clear();
        if active {
            self.last_click = None;
            self.cursor_inside = false;
            self.set_hover(Hover::None);
        }
    }

    fn apply_touch(&mut self, action: Action) {
        match action {
            Action::Tap((x, y)) => {
                self.on_mouse_move(x, y);
                self.on_mouse_down(MouseButton::Left);
                self.on_mouse_up(MouseButton::Left);
            }
            Action::Context((x, y)) => {
                self.last_click = None;
                self.on_mouse_move(x, y);
                self.on_mouse_down(MouseButton::Right);
                self.on_mouse_up(MouseButton::Right);
            }
            Action::Scroll { origin, delta } => {
                self.last_click = None;
                // 始终滚动落指的区域，手指越过侧栏、终端等边界也不会切换滚动对象。
                let mouse = self.mouse;
                self.mouse = origin;
                let scale = self.renderer.as_ref().map_or(1.0, Renderer::scale_factor);
                self.on_wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                    f64::from(delta.0 * scale),
                    f64::from(delta.1 * scale),
                )));
                self.mouse = mouse;
            }
            Action::Press((x, y)) => {
                self.last_click = None;
                self.on_mouse_move(x, y);
                self.on_mouse_down(MouseButton::Left);
            }
            Action::Move((x, y)) => self.on_mouse_move(x, y),
            Action::Release((x, y)) => {
                self.on_mouse_move(x, y);
                self.on_mouse_up(MouseButton::Left);
            }
            // 只有滚动条和分隔条会立即按下；收尾保存尺寸并结束拖动，不执行按钮或文件放置。
            Action::CancelDrag => self.on_mouse_up(MouseButton::Left),
        }
    }
}
