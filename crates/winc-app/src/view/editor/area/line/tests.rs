//! `line` 的测试：光标所在代码块的参考线。

use winc_editor::{Document, DocumentId};

use super::*;

#[test]
fn the_active_guide_is_the_block_around_the_caret() {
    let code = "fn a() {\n    if x {\n        one();\n\n        two();\n    }\n    three();\n}\n";
    let doc = Document::from_text(DocumentId(1), code);
    let text = doc.text();
    let lines = 0..text.line_count();
    let guide = |line: usize| active_guide(text, line, &lines, 4, 4);

    // 光标在块内时取所在的一层，空行包含在内，范围到最后一个非空行为止。
    assert_eq!(guide(2), Some((4, 2..5)));
    assert_eq!(guide(3), Some((4, 2..5)));
    // 光标在块的开头一行时取块内的一层。
    assert_eq!(guide(1), Some((4, 2..5)));
    assert_eq!(guide(0), Some((0, 1..7)));
    // 光标在外层语句上时取函数体的一层。
    assert_eq!(guide(6), Some((0, 1..7)));
    // 光标在最外层且不在块的开头时没有参考线。
    assert_eq!(guide(7), None);
    // 光标不在可见范围内时不查找。
    assert_eq!(active_guide(text, 2, &(4..8), 4, 4), None);
}
