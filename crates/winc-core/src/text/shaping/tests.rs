//! `needs_shaping` 的测试：组合符号和字形随上下文变化的文字需要整形，其余文字不需要。

use super::*;

#[test]
fn scripts_with_contextual_forms_need_shaping() {
    for c in [
        '\u{301}', 'ก', 'क', 'م', 'ש', 'ក', 'က', 'ཀ', 'ᠮ', '\u{1100}',
    ] {
        assert!(needs_shaping(c), "{c}");
    }
    for c in [
        'a', 'é', 'Ж', 'Ω', '中', 'あ', '한', '😀', '\u{200d}', '\u{fe0f}',
    ] {
        assert!(!needs_shaping(c), "{c}");
    }
}
