//! 词边界的测试：泰文、老挝文按词切开，其他文字和词内不切。

use super::*;

/// 返回按 [`word_breaks`] 切开后的各段。
fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    for at in word_breaks(text).into_iter().chain([text.len()]) {
        out.push(&text[from..at]);
        from = at;
    }
    out
}

#[test]
fn thai_and_lao_split_between_words() {
    assert_eq!(words("สวัสดีครับ"), ["สวัสดี", "ครับ"]);
    assert_eq!(words("ພາສາລາວ"), ["ພາສາ", "ລາວ"]);
}

#[test]
fn khmer_and_myanmar_split_somewhere() {
    assert!(words("ខ្ញុំស្រលាញ់ភាសាខ្មែរ").len() > 1);
    assert!(words("မြန်မာစာ").len() > 1);
}

#[test]
fn other_text_has_no_breaks() {
    assert!(word_breaks("hello world 中文").is_empty());
    // 泰文与其他文字之间的边界由排版自己的规则决定。
    assert!(word_breaks("abcไทย").is_empty());
}
