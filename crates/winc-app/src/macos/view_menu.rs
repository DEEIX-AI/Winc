//! 菜单栏的「视图」菜单：进入或退出全屏（`⌃⌘F`）。
//!
//! 全屏的快捷键来自菜单项的按键：`winit` 只建应用菜单，没有「视图」菜单时 `AppKit` 也不会
//! 自己补上一项，`⌃⌘F` 便作为普通按键送到窗口。菜单项用 `AppKit` 的标准动作
//! `toggleFullScreen:`、目标为空：动作沿响应链交给窗口，能否全屏、标题在「进入全屏幕」与
//! 「退出全屏幕」之间的切换都由系统按窗口状态处理。

use super::files::ns_string;
use super::*;

/// `NSEventModifierFlagControl`：`NSEventModifierFlags` 中的 ⌃。
const MOD_CONTROL: usize = 1 << 18;

/// `NSEventModifierFlagCommand`：`NSEventModifierFlags` 中的 ⌘。
const MOD_COMMAND: usize = 1 << 20;

/// 在菜单栏上加「视图」菜单，其中是「进入全屏幕  ⌃⌘F」，排在应用菜单之后、「窗口」菜单之前。
///
/// 必须在事件循环开始之后（`winit` 已建好菜单）调用；只有第一次调用生效。
pub fn add_view_menu() {
    static ADDED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if ADDED.set(()).is_err() {
        return;
    }
    let (Some(title), Some(fullscreen), Some(key)) = (
        ns_string(winc_i18n::t!("menu-view")),
        ns_string(winc_i18n::t!("menu-view-enter-full-screen")),
        ns_string("f"),
    ) else {
        return;
    };

    // SAFETY: 调用方在主线程调用（AppKit 的线程要求）。`NSApplication`、`NSMenu`、`NSMenuItem`
    // 是公开 API，返回的指针先判空再使用，插入下标不超过项数。`alloc` 出来的菜单和菜单项引用
    // 计数 +1，加进上一级后由上一级持有，`release` 抵消 `alloc`；对空指针发送消息是无操作。
    // 标题和快捷键字符串由 `ns_string` 创建并持有到本块结束，`AppKit` 会复制。
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let Some(app) = app.as_ref() else {
            return;
        };
        let main_menu: *mut AnyObject = msg_send![app, mainMenu];
        let Some(main_menu) = main_menu.as_ref() else {
            return;
        };

        let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
        let menu: *mut AnyObject = msg_send![menu, initWithTitle: &*title];
        let Some(menu) = menu.as_ref() else {
            return;
        };
        let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let item: *mut AnyObject = msg_send![
            item,
            initWithTitle: &*fullscreen,
            action: sel!(toggleFullScreen:),
            keyEquivalent: &*key
        ];
        if let Some(item) = item.as_ref() {
            // 菜单项按键默认带 ⌘，再加上 ⌃ 即系统的 ⌃⌘F。
            let () = msg_send![item, setKeyEquivalentModifierMask: MOD_CONTROL | MOD_COMMAND];
            let () = msg_send![menu, addItem: item];
        }
        let () = msg_send![item, release];

        let top: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
        let top: *mut AnyObject = msg_send![top, init];
        if let Some(top) = top.as_ref() {
            let () = msg_send![top, setTitle: &*title];
            let () = msg_send![top, setSubmenu: menu];
            // 第 0 项是应用菜单，本菜单排在其后。
            let count: isize = msg_send![main_menu, numberOfItems];
            let () = msg_send![main_menu, insertItem: top, atIndex: count.min(1)];
        }
        let () = msg_send![top, release];
        let () = msg_send![menu, release];
    }
}
