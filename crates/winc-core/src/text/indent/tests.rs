//! `indent` 模块的测试：空格与制表符的识别、跨多级退格与注释对齐的处理，以及缩进单位的换算。

use ropey::Rope;

use super::*;

fn guess(text: &str) -> Option<Indent> {
    detect(&Rope::from_str(text))
}

#[test]
fn spaces_tabs_and_nothing() {
    let four = "fn main() {\n    if x {\n        y();\n    }\n}\n";
    assert_eq!(guess(four), Some(Indent::Spaces(4)));

    let two = "a:\n  b:\n    c: 1\n  d: 2\n";
    assert_eq!(guess(two), Some(Indent::Spaces(2)));

    let tabs = "func main() {\n\tif x {\n\t\ty()\n\t}\n}\n";
    assert_eq!(guess(tabs), Some(Indent::Tabs));

    // 没有任何缩进行、以及空文本：无法判断。
    assert_eq!(guess("a\nb\n\nc\n"), None);
    assert_eq!(guess(""), None);
}

#[test]
fn outdenting_several_levels_does_not_fool_it() {
    // 4 个空格缩进，一次退两级时相邻两行相差 8；差值 8 出现次数更多时仍应识别为 4。
    let text = "class A:\n    def f(self):\n        if x:\n            pass\nprint()\n";
    assert_eq!(guess(text), Some(Indent::Spaces(4)));
}

#[test]
fn comment_alignment_and_blank_lines_are_ignored() {
    let text = "/**\n * doc\n * more\n */\nfn a() {\n\n  b();\n  if c {\n    d();\n  }\n}\n";
    assert_eq!(guess(text), Some(Indent::Spaces(2)));
}

#[test]
fn a_single_indented_level_uses_its_width() {
    // 只有一级缩进、没有可用的缩进差值时，取最小的缩进宽度。
    assert_eq!(guess("items:\n   - a\n   - b\n"), Some(Indent::Spaces(3)));
}

#[test]
fn units() {
    // `unit` 与 `of_unit` 都夹取到 1 至 8 个空格；制表符两向都能还原。
    assert_eq!(Indent::Tabs.unit(), "\t");
    assert_eq!(Indent::Spaces(2).unit(), "  ");
    assert_eq!(Indent::of_unit("    "), Indent::Spaces(4));
    assert_eq!(Indent::of_unit("\t"), Indent::Tabs);
}
