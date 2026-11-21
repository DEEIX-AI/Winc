//! 键盘按键映射的测试：macOS 编辑区的 Emacs 风格按键。

use super::*;

#[test]
fn emacs_keys_move_and_delete_like_in_fields() {
    let (a, e, f, b, n, p, d, h) = (
        emacs_command("a", false),
        emacs_command("e", false),
        emacs_command("f", false),
        emacs_command("b", false),
        emacs_command("n", false),
        emacs_command("p", false),
        emacs_command("d", false),
        emacs_command("h", false),
    );
    assert_eq!(a, Some(Command::LineStart { extend: false }));
    assert_eq!(e, Some(Command::LineEnd { extend: false }));
    assert_eq!(
        f,
        Some(Command::MoveRight {
            word: false,
            extend: false
        })
    );
    assert_eq!(
        b,
        Some(Command::MoveLeft {
            word: false,
            extend: false
        })
    );
    assert_eq!(n, Some(Command::MoveDown { extend: false }));
    assert_eq!(p, Some(Command::MoveUp { extend: false }));
    assert_eq!(d, Some(Command::Delete { word: false }));
    assert_eq!(h, Some(Command::Backspace { word: false }));
    assert_eq!(emacs_command("k", false), Some(Command::DeleteToLineEnd));
    assert_eq!(emacs_command("t", false), Some(Command::TransposeChars));
    assert_eq!(
        emacs_command("v", false),
        Some(Command::PageDown { extend: false })
    );
}

#[test]
fn emacs_keys_extend_the_selection_with_shift() {
    assert_eq!(
        emacs_command("a", true),
        Some(Command::LineStart { extend: true })
    );
    assert_eq!(
        emacs_command("p", true),
        Some(Command::MoveUp { extend: true })
    );
}

#[test]
fn emacs_keys_match_capital_letters_and_skip_others() {
    // 大写字母（大写锁定或 `Shift`）与对应的按键相同。
    assert_eq!(emacs_command("A", false), emacs_command("a", false));
    assert_eq!(
        emacs_command("N", true),
        Some(Command::MoveDown { extend: true })
    );
    // 其余的 `Ctrl` 组合不处理，也不当作输入。
    assert_eq!(emacs_command("x", false), None);
    assert_eq!(emacs_command("", false), None);
    assert_eq!(emacs_command("ab", false), None);
}
