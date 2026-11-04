//! 自动换行的性能测量：开着自动换行、每一行都被断开时，在文件中间输入一个字符的耗时。

use super::*;
use crate::{Command, DocumentId};
use std::time::Instant;

/// 在临时文档上重复插入，打印每次插入的平均耗时（毫秒）。
///
/// 用 `cargo test --release -p winc-editor measure_typing_with_wrap -- --ignored --nocapture`
/// 运行；每行插入 100 次后打印每次的平均耗时。
#[test]
#[ignore]
fn measure_typing_with_wrap() {
    for n in [10_000, 50_000, 200_000] {
        let text: String = (0..n)
            .map(|i| format!("line {i} lorem ipsum dolor sit amet consectetur adipiscing\n"))
            .collect();
        let mut d = Document::from_text(DocumentId(1), &text);
        d.ensure_wrap(Some(40), 4);
        d.selection = Selection::point(d.text().line_start(n / 2));
        let start = Instant::now();
        for _ in 0..100 {
            d.execute(Command::Insert("x".into()), 10, 4);
        }
        eprintln!("{n} 行：{:?} / 次", start.elapsed() / 100);
    }
}
