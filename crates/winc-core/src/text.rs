//! 行、列与显示列之间的换算、文本接口（`lines` 模块）、自动换行、缩进识别与需要整形的文字。
//!
//! `lines` 里的条目直接从这里导出（[`Lines`]、[`LineEnding`]），[`wrap`]、[`indent`] 在 crate 根上
//! 同样导出。

pub mod indent;
mod lines;
pub mod shaping;
pub mod wrap;

pub use lines::*;
