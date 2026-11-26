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

#[cfg(windows)]
#[test]
fn terminal_image_paste_uses_alt_v_for_an_image_without_text() {
    let mut app = App::new(
        winc_editor::Editor::default(),
        None,
        crate::bench::Bench::from_env(),
        winc_config::Config::default(),
    );
    let size = Size {
        cols: 80,
        rows: 24,
        ..Size::default()
    };
    let mut tab = TermTab::new(1, size);
    tab.modes.set(Modes::ALT_SCREEN, true);
    tab.modes.set(Modes::BRACKETED_PASTE, true);
    app.terminal.tabs.push(tab);

    assert_eq!(app.terminal_image_paste_key(true), None);
    app.clipboard.copied_image = Some(vec![1]);
    assert_eq!(
        app.terminal_image_paste_key(true).as_deref(),
        Some(&b"\x1bv"[..])
    );
    assert_eq!(app.terminal_image_paste_key(false), None);

    let tab = app.terminal.tab_mut().unwrap();
    tab.modes.set(Modes::BRACKETED_PASTE, false);
    assert_eq!(app.terminal_image_paste_key(true), None);
}
