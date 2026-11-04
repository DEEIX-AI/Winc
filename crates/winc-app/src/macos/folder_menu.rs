//! 目录菜单：点击状态栏路径时弹出的菜单。
//!
//! 子目录显示为子菜单，在 `AppKit` 即将显示子菜单时才读取其内容（见 [`menu_needs_update`]）。

use std::cell::RefCell;

use super::app_menu::{Action, DELEGATE, add_method, app_delegate};
use super::dialogs::{MenuIcon, add_separator, icon_image, menu_item, new_menu, pop_up};
use super::files::{file_url, url_path};
use super::*;

/// 目录菜单中的一项。
pub struct FolderItem {
    /// 菜单项的标题。
    pub title: String,
    /// 选中时打开的文件，或子菜单列出的目录；`None` 表示灰色的说明文字。
    pub path: Option<PathBuf>,
    /// 是否为目录（目录显示为子菜单）。
    pub is_dir: bool,
    /// 是否为当前文件，或通往当前文件的那一层目录；是时显示勾选标记。
    pub checked: bool,
    /// 标题前面的图标（16 × 16 逻辑像素，和文件树同一套）；没有时为 `None`。
    pub icon: Option<Image>,
}

/// 列出一个目录在菜单中的各项的函数：`None` 表示分隔线；两个参数依次为要列出的目录和当前
/// 文件（用于勾选）。
pub type ListFolder = fn(&Path, &Path) -> Vec<Option<FolderItem>>;

/// 目录菜单的回调：如何列出一个目录，以及选中文件后的动作；只在主线程上设置和调用。
struct Handlers {
    list: ListFolder,
    /// 选中文件时调用，参数是文件路径。
    chosen: Box<dyn Fn(PathBuf) + Send + Sync>,
}

static HANDLERS: OnceLock<Handlers> = OnceLock::new();

thread_local! {
    /// 正在显示的目录菜单对应的当前文件。
    ///
    /// 子菜单在菜单显示期间才读取，读取时用它来勾选。
    static CURRENT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// 子菜单尚未读取时占位项的 `tag`，值是 ASCII 字符串 `winc` 的四个字节。
///
/// 子菜单中先放一个占位项，是因为 `AppKit` 不一定会为空菜单询问委托。
const UNREAD: isize = 0x7769_6e63;

/// 选中文件时的动作名，添加在 `winit` 的应用委托上。
const OPEN_ACTION: &CStr = c"wincOpenFolderItem:";

/// 注册目录菜单的回调：`list` 列出一个目录，选中文件时以其路径调用 `chosen`。
///
/// 必须在事件循环开始之后调用；只有第一次调用生效。
pub fn on_folder_menu(list: ListFolder, chosen: impl Fn(PathBuf) + Send + Sync + 'static) {
    let handlers = Handlers {
        list,
        chosen: Box::new(chosen),
    };
    if HANDLERS.set(handlers).is_err() {
        return;
    }

    if let Some(delegate) = AnyClass::get(DELEGATE) {
        add_method(delegate, OPEN_ACTION, open_item as Action);
        add_method(delegate, c"menuNeedsUpdate:", menu_needs_update as Action);
    }
}

/// 在窗口的 `at`（逻辑坐标）处弹出 `dir` 的目录菜单，使打勾的那一项对准 `at`；菜单关闭后才返回。
///
/// `current` 是当前文件，用于勾选。
///
/// 返回是否弹出了菜单：回调尚未注册或无法创建菜单时返回 `false`。
pub fn popup_folder_menu(window: &Window, dir: &Path, current: &Path, at: (f64, f64)) -> bool {
    if HANDLERS.get().is_none() {
        return false;
    }
    let Some(menu) = new_menu() else {
        return false;
    };

    CURRENT.set(Some(current.to_path_buf()));
    // SAFETY: `menu` 是刚创建的有效 NSMenu，`fill` 返回的打勾项由它持有；调用方在主线程上
    // 调用（winit 只在主线程派发事件）。
    let shown = unsafe {
        let checked = fill(&menu, dir);
        pop_up(window, &menu, checked.as_deref(), at)
    };
    CURRENT.set(None);
    shown
}

/// 把 `dir` 中的各项加进 `menu`，返回第一个打勾的项。
///
/// 子目录先放一个只含占位项的子菜单，显示之前由 [`menu_needs_update`] 读取其内容。
///
/// # Safety
///
/// `menu` 必须指向有效的 NSMenu，并在主线程上调用。
unsafe fn fill(menu: &AnyObject, dir: &Path) -> Option<Retained<AnyObject>> {
    let handlers = HANDLERS.get()?;
    let current = CURRENT.with_borrow(Clone::clone)?;
    let delegate = app_delegate()?;
    let target: &AnyObject = &delegate;
    let open = Sel::register(OPEN_ACTION);
    let mut checked = None;

    // SAFETY: 由调用方保证 `menu` 有效；其余都是 NSMenu、NSMenuItem 的公开 API。新建的菜单项、
    // 子菜单加进去以后由菜单持有。子菜单的委托是应用委托，其存活时间长于任何菜单（整个事件
    // 循环期间）。
    unsafe {
        for item in (handlers.list)(dir, &current) {
            let Some(item) = item else {
                add_separator(menu);
                continue;
            };
            let action = (item.path.is_some() && !item.is_dir).then_some(open);
            let Some(entry) = menu_item(&item.title, action, "") else {
                continue;
            };

            match &item.path {
                None => {
                    let () = msg_send![&*entry, setEnabled: false];
                }
                Some(path) => {
                    let icon = item.icon.as_ref().and_then(|image| {
                        icon_image(&MenuIcon {
                            image,
                            size: (16.0, 16.0),
                        })
                    });
                    if let Some(icon) = icon {
                        let () = msg_send![&*entry, setImage: &*icon];
                    }
                    if let Some(url) = file_url(path, item.is_dir) {
                        let () = msg_send![&*entry, setRepresentedObject: &*url];
                    }
                    if item.is_dir {
                        if let Some(submenu) = unread_submenu(target) {
                            let () = msg_send![&*entry, setSubmenu: &*submenu];
                        }
                    } else {
                        let () = msg_send![&*entry, setTarget: target];
                    }
                }
            }

            if item.checked {
                // NSControlStateValueOn = 1。
                let () = msg_send![&*entry, setState: 1isize];
                checked.get_or_insert_with(|| entry.clone());
            }
            let () = msg_send![menu, addItem: &*entry];
        }
    }
    checked
}

/// 创建尚未读取的子菜单：只含一个占位项，委托设为 `delegate`。
fn unread_submenu(delegate: &AnyObject) -> Option<Retained<AnyObject>> {
    let submenu = new_menu()?;
    let placeholder = menu_item("…", None, "")?;
    // SAFETY: `submenu`、`placeholder` 都是刚创建的有效对象；占位项加进子菜单后由它持有；
    // `delegate` 是应用委托，存活时间长于子菜单。
    unsafe {
        let () = msg_send![&*placeholder, setTag: UNREAD];
        let () = msg_send![&*placeholder, setEnabled: false];
        let () = msg_send![&*submenu, addItem: &*placeholder];
        let () = msg_send![&*submenu, setDelegate: delegate];
    }
    Some(submenu)
}

/// `- (void)menuNeedsUpdate:(NSMenu *)menu`（NSMenuDelegate）：子菜单即将显示，读取目录中的内容。
///
/// 只有 [`unread_submenu`] 创建的子菜单把委托设成了本方法所在的应用委托；已经读过的子菜单不再
/// 读取。
extern "C-unwind" fn menu_needs_update(_this: *mut AnyObject, _cmd: Sel, menu: *mut AnyObject) {
    // SAFETY: `AppKit` 传入的是即将显示的 NSMenu，指针非空时有效。该菜单的上一级菜单中有一项
    // 以它为子菜单，那一项的 `representedObject` 是 `fill` 放入的 NSURL（`url_path` 要求参数
    // 是 NSURL）。回调在主线程上执行。
    unsafe {
        let Some(menu) = menu.as_ref() else {
            return;
        };
        let count: isize = msg_send![menu, numberOfItems];
        let first: *mut AnyObject = msg_send![menu, itemAtIndex: 0isize];
        let unread = count == 1
            && first.as_ref().is_some_and(|item| {
                let tag: isize = msg_send![item, tag];
                tag == UNREAD
            });
        if !unread {
            return;
        }

        let supermenu: *mut AnyObject = msg_send![menu, supermenu];
        let Some(supermenu) = supermenu.as_ref() else {
            return;
        };
        let index: isize = msg_send![supermenu, indexOfItemWithSubmenu: menu];
        let parent: *mut AnyObject = msg_send![supermenu, itemAtIndex: index];
        let Some(parent) = parent.as_ref() else {
            return;
        };
        let url: *mut AnyObject = msg_send![parent, representedObject];
        let Some(dir) = url.as_ref().and_then(|url| url_path(url)) else {
            return;
        };

        let () = msg_send![menu, removeAllItems];
        fill(menu, &dir);
    }
}

/// `- (void)wincOpenFolderItem:(NSMenuItem *)sender`：选中了一个文件，以其路径调用 `chosen` 回调。
extern "C-unwind" fn open_item(_this: *mut AnyObject, _cmd: Sel, sender: *mut AnyObject) {
    // SAFETY: 发送者是 `fill` 创建的 NSMenuItem，`representedObject` 是当时放入的 NSURL；
    // 指针为空时不读取（`as_ref` 返回 `None`）。
    let path = unsafe {
        sender.as_ref().and_then(|item| {
            let url: *mut AnyObject = msg_send![item, representedObject];
            url_path(url.as_ref()?)
        })
    };
    if let (Some(path), Some(handlers)) = (path, HANDLERS.get()) {
        (handlers.chosen)(path);
    }
}

#[cfg(test)]
mod tests;
