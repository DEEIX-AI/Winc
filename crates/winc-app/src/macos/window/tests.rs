//! `window` 的测试：`mouseDownCanMoveWindow` 的签名与 `NSView` 的声明一致，添加到子类后只
//! 生效一次且返回 `NO`。

use super::*;
use crate::macos::tests::method_encoding;

#[test]
fn title_bar_drags_reach_the_app() {
    // 添加的方法与 `NSView` 中的声明签名一致。
    assert_eq!(
        method_encoding(c"NSView", sel!(mouseDownCanMoveWindow), true),
        can_move_window_types().to_str().unwrap()
    );

    // 添加到新的 `NSView` 子类上：子类自身没有该方法（只继承了父类的实现），因此第一次
    // 添加成功；重复添加不做任何事。
    let class = objc2::runtime::ClassBuilder::new(c"WincTitleBarTestView", class!(NSView))
        .unwrap()
        .register();
    assert!(add_cannot_move_window(class));
    assert!(!add_cannot_move_window(class));
    let method = class.instance_method(sel!(mouseDownCanMoveWindow)).unwrap();
    // SAFETY: 方法是上面添加的 `cannot_move_window`，其签名与 `CanMoveWindow` 一致。
    let imp: CanMoveWindow = unsafe { std::mem::transmute(method.implementation()) };
    assert_eq!(
        imp(std::ptr::null_mut(), sel!(mouseDownCanMoveWindow)),
        Bool::NO
    );
}
