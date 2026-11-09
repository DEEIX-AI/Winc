//! 自绘标题栏的原生命中测试，让鼠标和触摸都由 Windows 移动、缩放及吸附窗口。

#![allow(unsafe_code)] // 同一窗口线程上的子类回调；共享数据由 Arc 和 Mutex 管理。

use std::sync::{Arc, Mutex};

use winc_render::Rect;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, GCL_STYLE, GetClassLongW, GetClientRect, HTBOTTOM, HTBOTTOMLEFT,
    HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed,
    SetClassLongW, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN,
    WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN,
    WM_XBUTTONDBLCLK, WM_XBUTTONDOWN,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

#[cfg(test)]
mod native_tests;

/// 顶栏中的控件保持客户区输入，空白处和窗口细边交给系统。矩形使用逻辑像素。
#[derive(Default)]
pub struct WindowHitTest {
    pub caption_height: f32,
    pub resize_border: f32,
    pub client: Vec<Rect>,
}

impl WindowHitTest {
    pub fn hit(&self, x: f32, y: f32, width: f32, height: f32, maximized: bool) -> u32 {
        if !Rect::new(0.0, 0.0, width, height).contains(x, y) {
            return HTCLIENT;
        }
        let edge = self.resize_border;
        if edge > 0.0 && !maximized {
            let (left, right) = (x < edge, x >= width - edge);
            let (top, bottom) = (y < edge, y >= height - edge);
            let hit = match (top, bottom, left, right) {
                (true, _, true, _) => HTTOPLEFT,
                (true, _, _, true) => HTTOPRIGHT,
                (_, true, true, _) => HTBOTTOMLEFT,
                (_, true, _, true) => HTBOTTOMRIGHT,
                (true, ..) => HTTOP,
                (_, true, ..) => HTBOTTOM,
                (_, _, true, _) => HTLEFT,
                (_, _, _, true) => HTRIGHT,
                _ => HTCLIENT,
            };
            if hit != HTCLIENT {
                return hit;
            }
        }
        if y < self.caption_height && !self.client.iter().any(|r| r.contains(x, y)) {
            HTCAPTION
        } else {
            HTCLIENT
        }
    }
}

/// 应用更新命中区域；窗口持有独立引用，销毁回调负责释放，不借用应用状态。
pub struct WindowInput(Arc<Mutex<WindowHitTest>>);

impl WindowInput {
    /// 在创建窗口的事件循环线程上安装；在窗口首次显示前调用。
    pub fn new(window: &Window) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let state = Arc::new(Mutex::new(WindowHitTest::default()));
        let data = Arc::into_raw(state.clone());
        // SAFETY: 当前线程创建并持有 window；回调只访问独立的共享数据，直到 WM_NCDESTROY。
        if unsafe { SetWindowSubclass(handle.hwnd.get() as _, Some(window_proc), 1, data as usize) }
            == 0
        {
            // SAFETY: 安装失败，没有回调持有这个引用。
            drop(unsafe { Arc::from_raw(data) });
            return None;
        }
        // SAFETY: winit 的窗口类默认不产生双击消息。启用后让 Windows 处理非客户区双击；
        // 本应用的每个窗口都安装此回调，客户区双击在回调中还原为 winit 所需的第二次按下。
        unsafe {
            let hwnd = handle.hwnd.get() as _;
            SetClassLongW(
                hwnd,
                GCL_STYLE,
                (GetClassLongW(hwnd, GCL_STYLE) | CS_DBLCLKS) as i32,
            );
        }
        Some(Self(state))
    }

    pub fn update(&self, hit: WindowHitTest) {
        if let Ok(mut state) = self.0.lock() {
            *state = hit;
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    if message == WM_NCLBUTTONDOWN && wparam == HTCAPTION as usize {
        // SAFETY: winit 在此消息中补发一个坐标为 (0, 0) 的鼠标移动；触摸提升的输入会把它
        // 当成真实移动，点按也可能触发吸附。直接交给系统，保留原始输入序列。
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    if message == WM_NCDESTROY {
        // SAFETY: data 是安装时交给该窗口的一个 Arc 引用；先移除回调，再释放且仅释放一次。
        unsafe {
            RemoveWindowSubclass(hwnd, Some(window_proc), id);
            drop(Arc::from_raw(data as *const Mutex<WindowHitTest>));
        }
    } else if message == WM_NCHITTEST {
        // SAFETY: 保留原生标题栏、边框等已有命中结果，只有客户区需要补充。
        let original = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if original != HTCLIENT as isize {
            return original;
        }
        // LPARAM 的两个坐标是有符号 16 位值，副屏可以位于主屏左边或上方。
        let mut point = POINT {
            x: lparam as i16 as i32,
            y: (lparam >> 16) as i16 as i32,
        };
        let mut bounds = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: hwnd 来自当前回调；两个输出都指向有效的栈变量。
        if unsafe { ScreenToClient(hwnd, &mut point) == 0 || GetClientRect(hwnd, &mut bounds) == 0 }
        {
            return original;
        }
        // SAFETY: 查询当前窗口，以实际 DPI 和尺寸处理系统拖动中的跨屏、最大化和缩放。
        let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
        // SAFETY: hwnd 是当前正在处理命中测试的窗口。
        let maximized = unsafe { IsZoomed(hwnd) } != 0;
        // SAFETY: 窗口持有此 Arc 引用；锁不会跨越任何可能重入回调的 Win32 调用。
        let state = unsafe { &*(data as *const Mutex<WindowHitTest>) };
        if let Ok(hit) = state.lock() {
            return hit.hit(
                point.x as f32 / scale,
                point.y as f32 / scale,
                bounds.right as f32 / scale,
                bounds.bottom as f32 / scale,
                maximized,
            ) as isize;
        }
        return original;
    }
    let message = match message {
        WM_LBUTTONDBLCLK => WM_LBUTTONDOWN,
        WM_MBUTTONDBLCLK => WM_MBUTTONDOWN,
        WM_RBUTTONDBLCLK => WM_RBUTTONDOWN,
        WM_XBUTTONDBLCLK => WM_XBUTTONDOWN,
        message => message,
    };
    // SAFETY: 其余消息原样沿子类链传递；WM_NCPOINTERDOWN 由系统提升为非客户区鼠标输入，
    // 继而进入系统拖窗循环，吸附预览、布局、还原尺寸和取消行为均由 Windows 决定。
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
