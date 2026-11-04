//! `macos` 各模块共用的测试辅助：读取 Objective-C 方法的类型编码，用来核对代码发出的消息与
//! 平台框架中的声明一致。

use super::*;

/// 返回方法类型编码：`class` 的实例方法（`instance` 为 `false` 时是类方法）`selector` 的
/// 返回值和参数类型依次拼接，并去掉其中表示栈偏移的数字。
///
/// 用于核对发出的消息与 `AppKit` 中的声明一致。
///
/// # Panics
///
/// `class` 未注册，或该类没有 `selector` 对应的方法时 panic。
pub(super) fn method_encoding(class: &CStr, selector: Sel, instance: bool) -> String {
    let class = AnyClass::get(class).unwrap();
    let method = if instance {
        class.instance_method(selector)
    } else {
        class.class_method(selector)
    };
    let method = method.unwrap_or_else(|| panic!("{class:?} 没有 {selector:?}"));
    let mut text = method.return_type().to_string_lossy().into_owned();
    for i in 0..method.arguments_count() {
        text.push_str(&method.argument_type(i).unwrap().to_string_lossy());
    }
    text.retain(|c| !c.is_ascii_digit());
    text
}
