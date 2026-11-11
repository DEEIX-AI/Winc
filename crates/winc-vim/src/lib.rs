//! Vim 模式的无界面部分：按键记法、命令表、命令解析器，以及命令在文本上的求值。
//!
//! 按键（[`Key`]）交给 [`Vim`]：解析器（[`Parser`]）按命令表（[`command::ENTRIES`]）把按键解析成
//! 命令（[`Command`]），命令在宿主（[`Host`]）提供的文本上求值，修改与选区交回宿主应用。寄存器、
//! 上一次修改等各窗口共用的状态在 [`Shared`] 中。[`fixture`] 是对照测试所用文件的格式。
//! 本 crate 不依赖窗口、渲染与编辑器。

pub mod command;
pub mod fixture;
pub mod host;
pub mod key;
mod motion;
mod operator;
pub mod parser;
pub mod register;
mod text;
mod vim;

pub use command::{Action, Entry, Force, Kind, Mode, Modes, Motion, Operator, Support, TextObject};
pub use host::{Host, Indentation, Input, Message};
pub use key::{Code, Key, Mods, Named, notation};
pub use parser::{Body, Command, Movement, Outcome, Parser, Target};
pub use register::{Clipboard, Registers};
pub use vim::{Shared, Vim, VisualKind};
