//! 词与词之间不写空格的文字（泰文、老挝文、高棉文、缅甸文）的词边界。
//!
//! 按 Unicode 换行算法（UAX #14）的词典扩展切分，由 `icu_segmenter` 完成；其他文字的可断处由
//! 排版自己的规则决定，这里只给出两侧都是这类文字的边界。

use std::cell::OnceCell;

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::LineBreakOptions;

/// 是否属于词与词之间不写空格的文字：泰文、老挝文、缅甸文（含扩展区）、高棉文（含符号区）。
pub(crate) fn spaceless(c: char) -> bool {
    matches!(c as u32,
        0x0E00..=0x0EFF
        | 0x1000..=0x109F | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F
        | 0x1780..=0x17FF | 0x19E0..=0x19FF)
}

/// 返回 `text` 中两侧都是 [`spaceless`] 文字的词边界（字节偏移，从小到大）。
///
/// 没有这类文字时不切分，返回空表；耗时与 `text` 的长度成正比，首次调用时载入词典。
pub(crate) fn word_breaks(text: &str) -> Vec<usize> {
    thread_local! {
        static SEGMENTER: OnceCell<icu_segmenter::LineSegmenterBorrowed<'static>> =
            const { OnceCell::new() };
    }
    if !text.chars().any(spaceless) {
        return Vec::new();
    }
    let between = |at: &usize| {
        let before = text[..*at].chars().next_back();
        let after = text[*at..].chars().next();
        before.is_some_and(spaceless) && after.is_some_and(spaceless)
    };
    SEGMENTER.with(|cell| {
        let segmenter =
            cell.get_or_init(|| LineSegmenter::new_dictionary(LineBreakOptions::default()));
        segmenter.segment_str(text).filter(between).collect()
    })
}

#[cfg(test)]
mod tests;
