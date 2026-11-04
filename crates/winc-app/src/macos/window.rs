//! 窗口外观与系统偏好的读取。
//!
//! - 红绿灯按钮在顶栏中垂直居中；顶栏中的标签可以拖动；双击顶栏的行为跟随系统偏好。
//! - 读取「减弱动态效果」和首选语言；查询鼠标此刻在窗口中的位置（拖文件进来时用）。
//! - 测量时窗口静默（透明、点击穿透、不抢占焦点）。

use super::files::rust_string;
use super::*;
use objc2::Encode;
use objc2::runtime::Bool;

/// `NSWindowButton` 的前三个值：关闭、最小化、缩放。
pub(super) const BUTTONS: [usize; 3] = [0, 1, 2];

/// 返回鼠标此刻在窗口内容区中的位置（逻辑像素，原点在左上角）；取不到时返回 `None`。
///
/// 从系统文件管理器拖文件进来时，系统不发鼠标移动事件，松手后的 `DroppedFile` 也不带位置；
/// 该事件在松手后立即处理，此时鼠标的位置就是落点。
pub fn mouse_location(window: &Window) -> Option<(f32, f32)> {
    let ns_window = ns_window(window)?;

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用。两个都是 NSWindow /
    // NSView 的公开只读方法，返回值是值类型的点和矩形；内容视图先判空再使用。
    unsafe {
        let point: CGPoint = msg_send![ns_window, mouseLocationOutsideOfEventStream];
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        let bounds: CGRect = msg_send![content.as_ref()?, bounds];
        // 窗口坐标的原点在左下角；内容视图铺满整个窗口（标题栏透明），高度相同。
        Some((point.x as f32, (bounds.size.height - point.y) as f32))
    }
}

/// 把窗口设为全透明、不画阴影、鼠标点击穿透到下方的窗口，用于测量。
///
/// 窗口仍然上屏并呈现（隐藏的窗口收不到重画事件，无法测量首帧），只是不可见、不可点击。
pub fn make_invisible(window: &Window) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用；三个调用都是
    // NSWindow 的公开 setter，参数是 `f64` 或 `bool` 值类型，不涉及指针所有权。
    unsafe {
        let () = msg_send![ns_window, setAlphaValue: 0.0f64];
        let () = msg_send![ns_window, setHasShadow: false];
        let () = msg_send![ns_window, setIgnoresMouseEvents: true];
    }
}

/// 是否有提示框或面板（sheet）挂在窗口上。
///
/// 弹出 sheet 时窗口会失去键盘焦点，但用户并没有离开窗口。
pub fn has_sheet(window: &Window) -> bool {
    let Some(ns_window) = ns_window(window) else {
        return false;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，调用方在主线程调用；`attachedSheet` 是
    // NSWindow 的公开 getter，返回可能为空的指针，只判空不解引用。
    unsafe {
        let sheet: *mut AnyObject = msg_send![ns_window, attachedSheet];
        !sheet.is_null()
    }
}

/// 把窗口显示到屏幕上但不使其成为键盘焦点窗口，用于测量。
///
/// `winit` 的 `set_visible(true)` 调用 `makeKeyAndOrderFront:`，会从当前前台应用处抢走键盘焦点。
pub fn show_without_focus(window: &Window) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };

    // SAFETY: `ns_window` 来自仍然存活的 winit 窗口，指向有效的 NSWindow；本函数只在主线程
    // 调用，满足 AppKit 的线程要求。`orderFrontRegardless` 只调整窗口层级，不激活应用，不
    // 抢占键盘焦点，也不转移窗口的所有权。
    unsafe {
        let () = msg_send![ns_window, orderFrontRegardless];
    }
}

/// 把红绿灯放进高 `bar_height` 的顶栏并垂直居中，第一个按钮距窗口左边缘 `left`（逻辑像素）；
/// 全屏时不调整。
///
/// 返回最后一个按钮的右边缘距窗口左边缘的距离（逻辑像素）。按钮的间距由系统决定（macOS 26
/// 为 23，更早的版本为 20），布局据此在红绿灯右侧留出空间。全屏或取不到按钮时返回 `None`。
pub fn place_traffic_lights(window: &Window, bar_height: f64, left: f64) -> Option<f64> {
    if window.fullscreen().is_some() {
        return None;
    }

    let ns_window = ns_window(window)?;

    // SAFETY: `ns_window` 是仍然存活的 winit 窗口背后的 NSWindow；winit 只在主线程上派发事件，
    // 本函数也在主线程调用。用到的都是 NSWindow / NSView 的公开方法，返回值先判空再使用。
    unsafe {
        let buttons = BUTTONS.map(|kind| {
            let button: *mut AnyObject = msg_send![ns_window, standardWindowButton: kind];
            button.as_ref()
        });
        let [Some(close), Some(minimize), Some(_)] = buttons else {
            return None;
        };
        let container = titlebar_container(close)?;

        // NSView 默认以左下角为原点：容器的底边放在窗口顶部向下 `bar_height` 处，使其占满顶栏。
        // `AppKit` 可能忽略设置的高度，但按钮相对容器底边摆放，结果相同。
        let window_frame: CGRect = msg_send![ns_window, frame];
        let mut frame: CGRect = msg_send![container, frame];
        frame.size.height = bar_height;
        frame.origin.y = window_frame.size.height - bar_height;
        let () = msg_send![container, setFrame: frame];

        let close_frame: CGRect = msg_send![close, frame];
        let minimize_frame: CGRect = msg_send![minimize, frame];
        let spacing = minimize_frame.origin.x - close_frame.origin.x;

        let mut end = left;
        for (i, button) in buttons.into_iter().flatten().enumerate() {
            let size: CGRect = msg_send![button, frame];
            let origin = CGPoint::new(
                left + i as f64 * spacing,
                ((bar_height - size.size.height) / 2.0).round(),
            );
            let () = msg_send![button, setFrameOrigin: origin];
            end = origin.x + size.size.width;
        }
        Some(end)
    }
}

/// 让顶栏中的标签可以拖动。
///
/// 标题栏透明、内容铺到标题栏下方时，`AppKit` 仍把标题栏那一条当作拖动窗口的区域：在那里
/// 按下并拖动，系统直接移动窗口，应用收不到拖动事件。本函数让内容视图对
/// `mouseDownCanMoveWindow` 返回 `NO`，使按下时不移动窗口；顶栏空白处由应用自己调用
/// `drag_window` 拖动窗口。必须在窗口显示之前调用。
pub fn keep_title_bar_drags(window: &Window) {
    if let Some(view) = ns_view(window) {
        add_cannot_move_window(view.class());
    }
}

type CanMoveWindow = extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool;

/// 实现 `- (BOOL)mouseDownCanMoveWindow`：固定返回 `NO`，窗口不随内容视图中的按下而移动。
extern "C-unwind" fn cannot_move_window(_this: *mut AnyObject, _cmd: Sel) -> Bool {
    Bool::NO
}

/// 返回值和参数的类型编码：BOOL、self、_cmd。
///
/// BOOL 在 Apple 芯片上是 `B`，在 Intel 上是 `c`。
fn can_move_window_types() -> CString {
    CString::new(format!("{}@:", Bool::ENCODING)).expect("类型编码里没有 NUL")
}

/// 给 `class` 添加返回 `NO` 的 `mouseDownCanMoveWindow` 方法，返回是否添加成功；类自身已有该
/// 方法时不做任何事并返回 `false`。
fn add_cannot_move_window(class: &AnyClass) -> bool {
    let types = can_move_window_types();
    // SAFETY: `CanMoveWindow` 的签名与 `can_move_window_types` 给出的编码一致：返回 BOOL，
    // 参数依次为 self（@）、_cmd（:）。两者同为 `extern "C-unwind"` 函数指针，大小相同。
    // 父类 `NSView` 的实现被该类覆盖，只影响该类的实例。
    unsafe {
        let imp = std::mem::transmute::<CanMoveWindow, Imp>(cannot_move_window);
        add_raw_method(class, sel!(mouseDownCanMoveWindow), imp, &types)
    }
}

/// 处理双击顶栏：按系统的「连按窗口标题栏」偏好（`AppleActionOnDoubleClick`）缩放或最小化窗口，
/// 偏好为「无操作」时不处理。
pub fn title_bar_double_click(window: &Window) {
    match action_on_double_click().as_deref() {
        Some("Minimize") => window.set_minimized(true),
        Some("None") => {}
        // 「缩放」「填充」和没有设置过都按缩放处理。
        _ => window.set_maximized(!window.is_maximized()),
    }
}

/// 用户是否在系统的辅助功能设置中打开了「减弱动态效果」。
pub fn reduce_motion() -> bool {
    // SAFETY: `NSWorkspace` 的 `sharedWorkspace` 和该只读属性都是公开 API，返回值先判空后使用。
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        workspace
            .as_ref()
            .is_some_and(|w| msg_send![w, accessibilityDisplayShouldReduceMotion])
    }
}

/// 返回用户在系统中排第一的首选语言（如 `zh-Hans-CN`、`en-US`）；无法取得时返回 `None`。
///
/// 从图形界面启动的应用没有 `LANG` 环境变量，因此需要向系统查询。
pub fn preferred_language() -> Option<String> {
    // SAFETY: NSLocale 的 `preferredLanguages` 是公开的类方法，返回 NSArray<NSString>；返回的
    // 对象先判空再使用，非空时是有效的 NSString，交给 `rust_string` 立即拷贝。
    unsafe {
        let languages: *mut AnyObject = msg_send![class!(NSLocale), preferredLanguages];
        let first: *mut AnyObject = msg_send![languages.as_ref()?, firstObject];
        rust_string(first.as_ref()?)
    }
}

/// 读取全局偏好 `AppleActionOnDoubleClick`，没有设置过时返回 `None`。
fn action_on_double_click() -> Option<String> {
    // SAFETY: NSUserDefaults 和 NSString 的类方法是线程安全的公开 API；传入的 C 字符串以 NUL 结尾；
    // 返回的对象先判空再使用；`value` 是有效的 NSString，交给 `rust_string` 立即拷贝。
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let defaults = defaults.as_ref()?;
        let name = c"AppleActionOnDoubleClick".as_ptr();
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: name];
        let key = key.as_ref()?;
        let value: *mut AnyObject = msg_send![defaults, stringForKey: key];
        rust_string(value.as_ref()?)
    }
}

/// 从按钮向上查找标题栏容器（`NSTitlebarContainerView`），找不到时返回 `None`。
///
/// 不同系统版本中间相隔的层数不同，因此最多向上查找 4 层。
///
/// # Safety
///
/// `view` 必须是有效的 NSView，并且在主线程上调用。
unsafe fn titlebar_container(view: &AnyObject) -> Option<&AnyObject> {
    let mut current = view;

    for _ in 0..4 {
        // SAFETY: 由调用方保证 `current` 是主线程上有效的 NSView，`superview` 可能为空。
        let parent: *mut AnyObject = unsafe { msg_send![current, superview] };
        // SAFETY: 父视图指针非空时指向有效的 NSView；本函数运行期间视图层级不会被修改，因此
        // 返回的引用与 `view` 的借用期同样有效。
        current = unsafe { parent.as_ref()? };

        if current
            .class()
            .name()
            .to_bytes()
            .ends_with(b"TitlebarContainerView")
        {
            return Some(current);
        }
    }

    None
}

#[cfg(test)]
mod tests;
