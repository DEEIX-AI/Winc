//! 菜单栏的「窗口」菜单和程序坞菜单：一个进程多个窗口时，`AppKit` 在这两处列出所有窗口
//! （按窗口标题），点一项切过去；⌘\` 在窗口之间循环切换也由 `AppKit` 处理。
//!
//! 「窗口」菜单交给 `NSApplication` 的 `windowsMenu` 以后，窗口列表由 `AppKit` 自己维护；程序坞
//! 菜单由应用委托的 `applicationDockMenu:` 返回，`AppKit` 在它上面加上窗口列表。

use super::app_menu::{DELEGATE, add_method};
use super::*;

/// 程序坞菜单里「新建窗口」被选中时调用的回调；只在主线程上设置和调用。
type NewWindowHandler = Box<dyn Fn() + Send + Sync>;

static NEW_WINDOW_HANDLER: OnceLock<NewWindowHandler> = OnceLock::new();

/// 程序坞菜单；建一次，一直留着（`applicationDockMenu:` 每次弹出都来要）。
struct DockMenu(Retained<AnyObject>);

// SAFETY: 菜单只在主线程上创建和读取（`applicationDockMenu:` 由 `AppKit` 在主线程调用）；
// 放进 `OnceLock` 只是为了留住它，不跨线程传递，也没有其他线程同时访问该对象。
unsafe impl Send for DockMenu {}
// SAFETY: `DockMenu` 只在主线程上读取，其中持有一个指向菜单的引用计数对象。`Sync` 允许的共享
// 访问只是让多个线程各持有一个指向同一菜单的不可变指针；实现中只在主线程创建和读取菜单，因此
// 不会出现并发的可变访问。
unsafe impl Sync for DockMenu {}

static DOCK_MENU: OnceLock<DockMenu> = OnceLock::new();

/// 程序坞菜单里「新建窗口」的动作名，添加在 `winit` 的应用委托上。
const NEW_WINDOW_ACTION: &CStr = c"wincNewWindow:";

/// 在菜单栏上加「窗口」菜单（最小化 ⌘M、缩放、全部前置，下面是 `AppKit` 列出的各窗口），并在
/// 程序坞菜单里加「新建窗口」，选中时调用 `new_window`。
///
/// 必须在事件循环开始之后（`winit` 已建好菜单和应用委托）调用；只有第一次调用生效。
pub fn add_window_menus(open: impl Fn() + Send + Sync + 'static) {
    if NEW_WINDOW_HANDLER.set(Box::new(open)).is_err() {
        return;
    }

    /// 实现 `- (void)wincNewWindow:(id)sender`：程序坞菜单中「新建窗口」的动作。
    extern "C-unwind" fn new_window(_this: *mut AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
        if let Some(open) = NEW_WINDOW_HANDLER.get() {
            open();
        }
    }

    /// 实现 `- (NSMenu *)applicationDockMenu:(NSApplication *)sender`：程序坞图标的菜单。
    extern "C-unwind" fn dock_menu(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
    ) -> *mut AnyObject {
        DOCK_MENU.get().map_or(std::ptr::null_mut(), |menu| {
            Retained::as_ptr(&menu.0).cast_mut()
        })
    }

    let Some(delegate) = AnyClass::get(DELEGATE) else {
        return;
    };
    add_method(delegate, NEW_WINDOW_ACTION, new_window);
    type DockMenuImp = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut AnyObject;
    // SAFETY: `DockMenuImp` 的签名与类型编码 `@@:@` 一致：返回对象（@），参数依次为 self（@）、
    // _cmd（:）、NSApplication（@）。两者同为 `extern "C-unwind"` 函数指针，大小相同。返回的菜单
    // 由 `DOCK_MENU` 一直持有，按 `AppKit` 的约定不转移所有权。
    unsafe {
        let imp = std::mem::transmute::<DockMenuImp, Imp>(dock_menu);
        add_raw_method(delegate, sel!(applicationDockMenu:), imp, c"@@:@");
    }

    // SAFETY: 本块用到的 `NSApplication`、`NSMenu`、`NSMenuItem`、`NSString` 都是公开 API；
    // 调用方保证在主线程调用。每个返回的指针先判空再使用；字符串都是以 NUL 结尾的 UTF-8 C
    // 字符串。`alloc` 出来的菜单和菜单项（引用计数 +1）加进上一级以后由上一级持有，随后
    // `release` 抵消 `alloc` 的那一次；程序坞菜单交给 `Retained` 持有。
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let Some(app) = app.as_ref() else {
            return;
        };
        // 菜单文字来自语言包，运行时转成 C 字符串；含 NUL 的文字（不会出现）按空字符串处理。
        let string = |text: &str| -> *mut AnyObject {
            let text = CString::new(text).unwrap_or_default();
            msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()]
        };
        let new_menu = |title: &str| -> Option<&AnyObject> {
            let menu: *mut AnyObject = msg_send![class!(NSMenu), alloc];
            let menu: *mut AnyObject = msg_send![menu, initWithTitle: string(title)];
            menu.as_ref()
        };
        // 动作发给第一响应者（`target` 为空）：最小化、缩放、全部前置都是 `NSWindow`、
        // `NSApplication` 自己的动作。
        let add_item = |menu: &AnyObject, title: &str, action: Sel, key: &str| {
            let item: *mut AnyObject = msg_send![
                menu,
                addItemWithTitle: string(title),
                action: action,
                keyEquivalent: string(key)
            ];
            item
        };

        // 「窗口」菜单。
        let main_menu: *mut AnyObject = msg_send![app, mainMenu];
        if let Some(main_menu) = main_menu.as_ref()
            && let Some(menu) = new_menu(winc_i18n::t!("menu-window"))
        {
            add_item(
                menu,
                winc_i18n::t!("menu-window-minimize"),
                sel!(performMiniaturize:),
                "m",
            );
            add_item(
                menu,
                winc_i18n::t!("menu-window-zoom"),
                sel!(performZoom:),
                "",
            );
            let separator: *mut AnyObject = msg_send![class!(NSMenuItem), separatorItem];
            let () = msg_send![menu, addItem: separator];
            add_item(
                menu,
                winc_i18n::t!("menu-window-bring-all-to-front"),
                sel!(arrangeInFront:),
                "",
            );

            let item: *mut AnyObject = msg_send![class!(NSMenuItem), alloc];
            let item: *mut AnyObject = msg_send![item, init];
            if let Some(item) = item.as_ref() {
                let () = msg_send![item, setTitle: string(winc_i18n::t!("menu-window"))];
                let () = msg_send![item, setSubmenu: menu];
                let () = msg_send![main_menu, addItem: item];
                let () = msg_send![app, setWindowsMenu: menu];
                let () = msg_send![item, release];
            }
            let () = msg_send![menu, release];
        }

        // 程序坞菜单：「新建窗口」，上面由 `AppKit` 列出各窗口。
        let Some(menu) = new_menu("") else {
            return;
        };
        let item = add_item(
            menu,
            winc_i18n::t!("cmd-new-window"),
            Sel::register(NEW_WINDOW_ACTION),
            "",
        );
        if let Some(item) = item.as_ref() {
            let target: *mut AnyObject = msg_send![app, delegate];
            let () = msg_send![item, setTarget: target];
        }
        if let Some(menu) = Retained::from_raw((menu as *const AnyObject).cast_mut()) {
            let _ = DOCK_MENU.set(DockMenu(menu));
        }
    }
}
