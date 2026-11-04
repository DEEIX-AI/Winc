//! `keys` 的测试：非拉丁键盘布局按物理键位取字符。

use super::*;
use winit::keyboard::KeyCode;

#[test]
fn control_on_a_non_latin_layout_uses_the_key_position() {
    assert_eq!(latin('с', PhysicalKey::Code(KeyCode::KeyC)), 'c');
    assert_eq!(latin('х', PhysicalKey::Code(KeyCode::BracketLeft)), '[');
    // 拉丁字母的键盘（法文 AZERTY 的 A 在 Q 的位置上）按按键上的字符处理。
    assert_eq!(latin('a', PhysicalKey::Code(KeyCode::KeyQ)), 'a');
    assert_eq!(latin('é', PhysicalKey::Code(KeyCode::F1)), 'é');
}
