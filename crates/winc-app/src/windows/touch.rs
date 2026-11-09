//! 自绘标题栏和窗口边缘的触摸拖动。
//!
//! winit 的系统拖动接口依赖鼠标按键，不能接管客户端触摸。这里读取接触点的屏幕坐标，按落指
//! 时的窗口位置和大小计算变化；窗口移动后客户端坐标会改变，不能用它累计位移。

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::Touch;
use winit::window::{ResizeDirection, Window};

#[cfg(test)]
mod native_tests;

/// 用一根手指移动窗口或拖动其边缘；位置和尺寸均为物理像素。
pub struct TouchWindowDrag {
    pointer: u64,
    start: PhysicalPosition<i32>,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    frame: PhysicalSize<u32>,
    minimum: PhysicalSize<u32>,
    direction: Option<ResizeDirection>,
    maximized: bool,
    moved: bool,
}

impl TouchWindowDrag {
    /// 在落指时记下窗口几何；`direction` 为 `None` 时移动，否则调整对应边缘。
    ///
    /// `minimum` 是客户区的最小尺寸。拖动最大化窗口的标题栏时先还原，保持手指位于标题栏
    /// 相同的横向比例处。无法读取窗口位置时返回 `None`。
    pub fn start(
        window: &Window,
        touch: &Touch,
        direction: Option<ResizeDirection>,
        minimum: PhysicalSize<u32>,
    ) -> Option<Self> {
        let start = screen_point(window, touch)?;
        let size = window.outer_size();
        let inner = window.inner_size();
        Some(Self {
            pointer: touch.id,
            start,
            position: window.outer_position().ok()?,
            size,
            frame: PhysicalSize::new(
                size.width.saturating_sub(inner.width),
                size.height.saturating_sub(inner.height),
            ),
            minimum,
            direction,
            maximized: window.is_maximized(),
            moved: false,
        })
    }

    /// 按接触点当前的屏幕位置更新窗口；其他手指和无法读取的位置不产生移动。
    pub fn update(&mut self, window: &Window, touch: &Touch) {
        if touch.id != self.pointer {
            return;
        }
        let Some(point) = screen_point(window, touch) else {
            return;
        };
        if !self.moved {
            let distance =
                f64::from(point.x - self.start.x).hypot(f64::from(point.y - self.start.y));
            if distance <= 8.0 * window.scale_factor() {
                return;
            }
            // 真正拖动后才还原；单击或双击最大化窗口的标题栏不会先意外还原。
            if self.maximized && self.direction.is_none() {
                let fraction =
                    f64::from(self.start.x - self.position.x) / f64::from(self.size.width.max(1));
                let top = self.start.y - self.position.y;
                window.set_maximized(false);
                self.size = window.outer_size();
                let inner = window.inner_size();
                self.frame = PhysicalSize::new(
                    self.size.width.saturating_sub(inner.width),
                    self.size.height.saturating_sub(inner.height),
                );
                self.position = PhysicalPosition::new(
                    point.x
                        - (fraction.clamp(0.0, 1.0) * f64::from(self.size.width)).round() as i32,
                    point.y - top,
                );
                self.start = point;
            }
            self.moved = true;
        }
        let (position, size) = self.geometry(point);
        if self.direction.is_some() {
            let _ = window.request_inner_size(size);
        }
        window.set_outer_position(position);
    }

    fn geometry(&self, point: PhysicalPosition<i32>) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
        let dx = point.x - self.start.x;
        let dy = point.y - self.start.y;
        let mut position = self.position;
        let mut size = self.size;
        use ResizeDirection::*;
        if let Some(direction) = self.direction {
            let west = matches!(direction, West | NorthWest | SouthWest);
            let north = matches!(direction, North | NorthWest | NorthEast);
            if west || matches!(direction, East | NorthEast | SouthEast) {
                let width = self.size.width as i32 + if west { -dx } else { dx };
                size.width = width.max((self.minimum.width + self.frame.width) as i32) as u32;
                if west {
                    position.x += self.size.width as i32 - size.width as i32;
                }
            }
            if north || matches!(direction, South | SouthWest | SouthEast) {
                let height = self.size.height as i32 + if north { -dy } else { dy };
                size.height = height.max((self.minimum.height + self.frame.height) as i32) as u32;
                if north {
                    position.y += self.size.height as i32 - size.height as i32;
                }
            }
        } else {
            position.x += dx;
            position.y += dy;
        }
        (
            position,
            PhysicalSize::new(
                size.width - self.frame.width,
                size.height - self.frame.height,
            ),
        )
    }
}

/// 优先从当前指针读取屏幕坐标，避免窗口移动后，队列中旧事件的客户端坐标造成来回跳动。
#[allow(unsafe_code)] // GetPointerInfo
fn screen_point(window: &Window, touch: &Touch) -> Option<PhysicalPosition<i32>> {
    use windows_sys::Win32::UI::Input::Pointer::{GetPointerInfo, POINTER_INFO};
    let mut info = std::mem::MaybeUninit::<POINTER_INFO>::uninit();
    if let Ok(id) = u32::try_from(touch.id)
        // SAFETY: info 指向足够大的可写结构；仅在成功填充后读取，接触点编号来自 winit。
        && unsafe { GetPointerInfo(id, info.as_mut_ptr()) } != 0
    {
        // SAFETY: GetPointerInfo 已成功初始化整个结构。
        let point = unsafe { info.assume_init() }.ptPixelLocation;
        return Some(PhysicalPosition::new(point.x, point.y));
    }
    let origin = window.inner_position().ok()?;
    Some(PhysicalPosition::new(
        origin.x + touch.location.x.round() as i32,
        origin.y + touch.location.y.round() as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_window_resize_keeps_opposite_edges_fixed_at_the_minimum() {
        let drag = TouchWindowDrag {
            pointer: 1,
            start: PhysicalPosition::new(-100, -100),
            position: PhysicalPosition::new(-500, -200),
            size: PhysicalSize::new(800, 600),
            frame: PhysicalSize::new(0, 0),
            minimum: PhysicalSize::new(400, 300),
            direction: Some(ResizeDirection::NorthWest),
            maximized: false,
            moved: false,
        };
        assert_eq!(
            drag.geometry(PhysicalPosition::new(1000, 1000)),
            (
                PhysicalPosition::new(-100, 100),
                PhysicalSize::new(400, 300)
            )
        );
    }
}
