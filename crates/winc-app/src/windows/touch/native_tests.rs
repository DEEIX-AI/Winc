//! 需要交互桌面的系统测试：向临时测试窗口注入触摸，验证窗口位置实际变化。

#![allow(unsafe_code)] // Windows 触摸注入，仅作用于本测试创建的窗口。

use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::UI::Input::Pointer::{
    InitializeTouchInjection, InjectTouchInput, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_TOUCH_INFO,
    TOUCH_FEEDBACK_NONE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PT_TOUCH, TOUCH_MASK_CONTACTAREA, TOUCH_MASK_ORIENTATION, TOUCH_MASK_PRESSURE, WindowFromPoint,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::windows::EventLoopBuilderExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId, WindowLevel};

use super::*;

#[test]
#[ignore = "injects touch into a temporary visible window; requires an interactive Windows desktop"]
fn native_touch_moves_the_window_without_a_mouse_button() {
    struct TestApp {
        window: Option<Window>,
        origin: Option<PhysicalPosition<i32>>,
        deadline: Instant,
        proxy: EventLoopProxy<()>,
        injection: Arc<Mutex<Option<io::Result<()>>>>,
        moved: bool,
        drag: Option<TouchWindowDrag>,
    }
    impl ApplicationHandler<()> for TestApp {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let window = event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Winc touch drag test")
                        .with_decorations(false)
                        .with_active(false)
                        .with_window_level(WindowLevel::AlwaysOnTop)
                        .with_position(PhysicalPosition::new(200, 200))
                        .with_inner_size(PhysicalSize::new(400, 240)),
                )
                .unwrap();
            let origin = window.outer_position().unwrap();
            let client = window.inner_position().unwrap();
            let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() else {
                unreachable!()
            };
            let hwnd = handle.hwnd.get();
            self.origin = Some(origin);
            self.window = Some(window);
            let proxy = self.proxy.clone();
            let result = self.injection.clone();
            std::thread::spawn(move || {
                let injected = inject_drag(hwnd, client.x + 100, client.y + 24);
                *result.lock().unwrap() = Some(injected);
                let _ = proxy.send_event(());
            });
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            if let WindowEvent::Touch(touch) = event {
                let window = self.window.as_ref().unwrap();
                match touch.phase {
                    TouchPhase::Started => {
                        self.drag = TouchWindowDrag::start(
                            window,
                            &touch,
                            None,
                            PhysicalSize::new(100, 100),
                        )
                    }
                    TouchPhase::Moved => {
                        if let Some(drag) = &mut self.drag {
                            drag.update(window, &touch);
                        }
                    }
                    _ => self.drag = None,
                }
            }
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, _: ()) {
            let at = self.window.as_ref().unwrap().outer_position().unwrap();
            let origin = self.origin.unwrap();
            self.moved = at.x - origin.x >= 80 && at.y - origin.y >= 30;
            event_loop.exit();
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if Instant::now() >= self.deadline {
                event_loop.exit();
            }
        }
    }
    let event_loop = EventLoop::<()>::with_user_event()
        .with_any_thread(true)
        .build()
        .unwrap();
    let mut app = TestApp {
        window: None,
        origin: None,
        deadline: Instant::now() + Duration::from_secs(5),
        proxy: event_loop.create_proxy(),
        injection: Arc::default(),
        moved: false,
        drag: None,
    };
    event_loop.run_app(&mut app).unwrap();
    app.injection
        .lock()
        .unwrap()
        .take()
        .expect("injection completed")
        .unwrap();
    assert!(
        app.moved,
        "native touch must move the window, without pressing a mouse button"
    );
}

fn inject_drag(hwnd: isize, x: i32, y: i32) -> io::Result<()> {
    std::thread::sleep(Duration::from_millis(150));
    // SAFETY: WindowFromPoint 只查询该点；先确认触摸不会命中其他应用。
    if unsafe { WindowFromPoint(POINT { x, y }) } as isize != hwnd {
        return Err(io::Error::other("test window is covered"));
    }
    // SAFETY: 初始化当前进程的单点触摸注入，无指针参数。
    if unsafe { InitializeTouchInjection(1, TOUCH_FEEDBACK_NONE) } == 0 {
        return Err(io::Error::other(format!(
            "InitializeTouchInjection: {}",
            io::Error::last_os_error()
        )));
    }
    // SAFETY: POINTER_TOUCH_INFO 是 Win32 的纯数据结构，全零为未设置的可选字段。
    let mut contact: POINTER_TOUCH_INFO = unsafe { std::mem::zeroed() };
    contact.pointerInfo.pointerType = PT_TOUCH;
    contact.pointerInfo.pointerId = 0;
    contact.pointerInfo.ptPixelLocation = POINT { x, y };
    contact.touchMask = TOUCH_MASK_CONTACTAREA | TOUCH_MASK_ORIENTATION | TOUCH_MASK_PRESSURE;
    contact.orientation = 90;
    contact.pressure = 320;
    let send = |contact: &mut POINTER_TOUCH_INFO, flags| -> io::Result<()> {
        contact.pointerInfo.pointerFlags = flags;
        let p = contact.pointerInfo.ptPixelLocation;
        contact.rcContact = RECT {
            left: p.x - 2,
            top: p.y - 2,
            right: p.x + 2,
            bottom: p.y + 2,
        };
        // SAFETY: 指向一个完整有效的接触点结构，数组长度为 1；调用返回后不再引用它。
        if unsafe { InjectTouchInput(1, contact) } == 0 {
            Err(io::Error::other(format!(
                "InjectTouchInput ({flags:#x}, {}, {}): {}",
                p.x,
                p.y,
                io::Error::last_os_error()
            )))
        } else {
            Ok(())
        }
    };
    let active = POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT;
    send(&mut contact, active | POINTER_FLAG_DOWN)?;
    let moved = (1..=5).try_for_each(|step| {
        std::thread::sleep(Duration::from_millis(30));
        contact.pointerInfo.ptPixelLocation = POINT {
            x: x + step * 20,
            y: y + step * 10,
        };
        send(&mut contact, active | POINTER_FLAG_UPDATE)
    });
    std::thread::sleep(Duration::from_millis(30));
    // 即使中间注入失败也抬手，避免留下接触中的指针。
    let released = send(&mut contact, POINTER_FLAG_UP);
    std::thread::sleep(Duration::from_millis(100));
    moved.and(released)
}
