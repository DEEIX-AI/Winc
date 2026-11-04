//! 文本内核：位置换算、显示列、选区、光标移动、编辑事务与撤销、逐行差分、整理文字的命令。
//!
//! 所有位置都是 `char` 偏移，行号从 0 起。文本通过 [`Lines`] 只读访问，编辑通过
//! [`EditBuffer`](text::EditBuffer) 应用，一次用户操作产生一个 [`Transaction`]，由 [`History`]
//! 记录成一个撤销单元。换行只认 LF、CRLF、CR 三种（[`LineEnding`]）。[`graph`] 与 [`hash`]
//! 分别提供提交图和 FNV-1a 哈希，与文本内容无关。

pub mod diff;
mod editing;
pub mod graph;
pub mod hash;
pub mod text;

pub use editing::edit::{Change, EditKind, History, Restored, Transaction};
pub use editing::selection::Selection;
pub use editing::{edit, movement, selection, transform};
pub use text::indent::Indent;
pub use text::{Line, LineEnding, Lines};
pub use text::{indent, wrap};
