//! 编辑区的行为：编辑命令、多光标、折叠、粘滞滚动、查找、转到定义、符号、后退前进、书签、
//! 粘贴、格式化、滚动条、提示气泡和磁盘冲突。
//!
//! 每个子模块负责一类命令，由 [`App`] 上对应的方法调用；文档状态保存在 [`App`] 的字段里。

use super::*;

pub(super) mod bookmarks;
pub(super) mod conflict;
pub(super) mod cursors;
pub(crate) mod definition;
pub(super) mod editing;
pub(super) mod find;
pub(super) mod folding;
pub(super) mod format;
pub(super) mod hint;
pub(super) mod navigation;
pub(super) mod paste;
pub(super) mod scrollbar;
pub(super) mod sticky;
pub(super) mod symbols;
