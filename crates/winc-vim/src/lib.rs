//! Vim 模式的无界面部分：按键记法、命令表与命令解析器。
//!
//! 按键（[`Key`]）由 [`Parser`] 逐个解析成命令（[`Command`]）：寄存器、次数、操作符、动作或
//! 文本对象。命令的定义集中在命令表（[`command::ENTRIES`]），每条命令一条记录。[`fixture`]
//! 是对照测试所用文件的格式。本 crate 不依赖窗口、渲染与编辑器，不求值命令。

pub mod command;
pub mod fixture;
pub mod key;
pub mod parser;

pub use command::{Action, Entry, Force, Kind, Mode, Modes, Motion, Operator, Support, TextObject};
pub use key::{Code, Key, Mods, Named, notation};
pub use parser::{Body, Command, Movement, Outcome, Parser, Target};
