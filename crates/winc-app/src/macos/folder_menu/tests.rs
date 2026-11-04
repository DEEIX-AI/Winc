//! `folder_menu` 的测试：目录菜单发出的每条 `AppKit` 消息都存在，且参数编码与传入类型一致。

use super::*;
use crate::macos::tests::method_encoding;

#[test]
fn folder_menu_messages_match_appkit_signatures() {
    // 菜单不能在测试中弹出（会显示在屏幕上），因此核对用到的每条消息在 AppKit 中确实存在，
    // 且参数类型与传入的一致。
    for name in [
        sel!(setRepresentedObject:),
        sel!(setSubmenu:),
        sel!(setTarget:),
    ] {
        assert_eq!(method_encoding(c"NSMenuItem", name, true), "v@:@");
    }
    assert_eq!(
        method_encoding(c"NSMenuItem", sel!(representedObject), true),
        "@@:"
    );
    assert_eq!(
        method_encoding(c"NSMenuItem", sel!(setState:), true),
        "v@:q"
    );
    assert_eq!(method_encoding(c"NSMenu", sel!(setDelegate:), true), "v@:@");
    assert_eq!(method_encoding(c"NSMenu", sel!(numberOfItems), true), "q@:");
    assert_eq!(method_encoding(c"NSMenu", sel!(itemAtIndex:), true), "@@:q");
    assert_eq!(method_encoding(c"NSMenu", sel!(supermenu), true), "@@:");
    assert_eq!(
        method_encoding(c"NSMenu", sel!(indexOfItemWithSubmenu:), true),
        "q@:@"
    );
    assert_eq!(
        method_encoding(c"NSMenu", sel!(removeAllItems), true),
        "v@:"
    );

    // `menuNeedsUpdate:` 是 NSMenuDelegate 的可选实例方法，签名与添加的 `v@:@` 一致。
    let protocol = objc2::runtime::AnyProtocol::get(c"NSMenuDelegate").unwrap();
    // SAFETY: 协议来自运行时；返回的类型编码是运行时持有的 C 字符串，此处立即拷贝。
    let mut types = unsafe {
        let method = ffi::protocol_getMethodDescription(
            protocol,
            sel!(menuNeedsUpdate:),
            objc2::runtime::Bool::NO,
            objc2::runtime::Bool::YES,
        );
        assert_eq!(method.name, Some(sel!(menuNeedsUpdate:)));
        CStr::from_ptr(method.types).to_string_lossy().into_owned()
    };
    types.retain(|c| !c.is_ascii_digit());
    assert_eq!(types, "v@:@");
}
