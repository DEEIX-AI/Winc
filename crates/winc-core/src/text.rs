//! 行、列与显示列之间的换算、文本接口（`lines` 模块）、自动换行与缩进识别。
//!
//! `lines` 里的条目直接从这里导出（[`Lines`]、[`LineEnding`]），[`wrap`]、[`indent`] 在 crate 根上
//! 同样导出。

pub mod indent;
mod lines;
pub mod wrap;

pub use lines::*;
