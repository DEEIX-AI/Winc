//! 命令表：每条命令一条记录（[`Entry`]），包含模式、按键序列、种类与支持状态。
//!
//! [`ENTRIES`] 是全部记录，首次使用时按模式建成前缀树，供解析器逐键查找。操作符的叠写形式
//! （`dd`、`g~~`）不单独列出，由解析器按操作符的按键序列识别。

mod table;

pub use table::ENTRIES;
pub(crate) use table::{Lookup, ROOT, Table};

/// 命令表区分的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// 普通模式。
    Normal,
    /// 可视模式，包括按字符、按行、按块三种。
    Visual,
    /// 等待动作：普通模式下输入操作符之后，到动作或文本对象输入完成之前。
    OperatorPending,
    /// 插入模式。
    Insert,
}

impl Mode {
    /// 全部模式。
    pub const ALL: [Self; 4] = [
        Self::Normal,
        Self::Visual,
        Self::OperatorPending,
        Self::Insert,
    ];

    /// 返回模式在 [`Mode::ALL`] 中的下标。
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// 一组模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modes(u8);

impl Modes {
    /// 普通模式。
    pub const N: Self = Self::of(Mode::Normal);
    /// 可视模式。
    pub const V: Self = Self::of(Mode::Visual);
    /// 等待动作。
    pub const O: Self = Self::of(Mode::OperatorPending);
    /// 插入模式。
    pub const I: Self = Self::of(Mode::Insert);
    /// 普通与可视模式。
    pub const NV: Self = Self::N.with(Self::V);
    /// 可视模式与等待动作。
    pub const VO: Self = Self::V.with(Self::O);
    /// 普通、可视模式与等待动作。
    pub const NVO: Self = Self::NV.with(Self::O);

    /// 返回只含 `mode` 的一组模式。
    pub const fn of(mode: Mode) -> Self {
        Self(1 << mode.index())
    }

    /// 返回两组模式的并集。
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// 是否包含 `mode`。
    pub const fn contains(self, mode: Mode) -> bool {
        self.0 & Self::of(mode).0 != 0
    }
}

/// 动作：把光标移到另一个位置的命令，可以作为操作符的范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    /// `h`：向左一个字符。
    Left,
    /// `l`：向右一个字符。
    Right,
    /// `k`：向上一行。
    Up,
    /// `j`：向下一行。
    Down,
    /// `w`：下一个词的开头。
    WordForward,
    /// `b`：上一个词的开头。
    WordBackward,
    /// `e`：词尾。
    WordEnd,
    /// `ge`：上一个词的词尾。
    WordEndBackward,
    /// `0`：行首。
    LineStart,
    /// `^`：行内第一个非空白字符。
    FirstNonBlank,
    /// `$`：行尾。
    LineEnd,
    /// `gg`：第一行，带次数时为指定的行。
    FirstLine,
    /// `G`：最后一行，带次数时为指定的行。
    LastLine,
    /// `f{字符}`：行内向右查找字符。
    FindForward,
    /// `F{字符}`：行内向左查找字符。
    FindBackward,
    /// `t{字符}`：行内向右查找字符，停在该字符之前。
    TillForward,
    /// `T{字符}`：行内向左查找字符，停在该字符之后。
    TillBackward,
    /// `;`：重复上一次行内查找。
    RepeatFind,
    /// `,`：反方向重复上一次行内查找。
    RepeatFindReverse,
}

/// 操作符：作用于一个动作、文本对象或选区所覆盖的文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// `d`：删除。
    Delete,
    /// `c`：删除并进入插入模式。
    Change,
    /// `y`：复制。
    Yank,
    /// `>`：增加缩进。
    Indent,
    /// `<`：减少缩进。
    Outdent,
    /// `g~`：切换大小写。
    ToggleCase,
    /// `gu`：转为小写。
    Lowercase,
    /// `gU`：转为大写。
    Uppercase,
    /// `gq`：重排文本。
    Format,
}

/// 文本对象的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    /// `w`：词。
    Word,
    /// `(`、`)`、`b`：圆括号。
    Paren,
    /// `"`：双引号。
    DoubleQuote,
}

/// 文本对象：以 `i` 或 `a` 开头，选中光标处的一段文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextObject {
    /// 形状。
    pub shape: Shape,
    /// 是否包含两侧的空白或定界符：`a` 开头为真，`i` 开头为假。
    pub around: bool,
}

/// 不带范围的命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// `x`：删除光标处的字符。
    DeleteChar,
    /// `X`：删除光标之前的字符。
    DeleteCharBefore,
    /// `D`：删除到行尾。
    DeleteToLineEnd,
    /// `C`：删除到行尾并进入插入模式。
    ChangeToLineEnd,
    /// `Y`：复制整行。
    YankLine,
    /// `p`：粘贴到光标之后。
    PasteAfter,
    /// `P`：粘贴到光标之前。
    PasteBefore,
    /// `r{字符}`：把光标处的字符替换为给定的字符。
    ReplaceChar,
    /// `u`：撤销。
    Undo,
    /// `Ctrl-R`：重做。
    Redo,
    /// `.`：重复上一次修改。
    Repeat,
    /// `i`：在光标之前进入插入模式。
    Insert,
    /// `a`：在光标之后进入插入模式。
    Append,
    /// `v`：进入或离开按字符的可视模式。
    Visual,
    /// `V`：进入或离开按行的可视模式。
    VisualLine,
    /// `Ctrl-V`、`Ctrl-Q`：进入或离开按块的可视模式。
    VisualBlock,
    /// `m{字符}`：设置标记。
    SetMark,
    /// `<Esc>`：离开可视模式或插入模式。
    Escape,
    /// 插入模式的 `Ctrl-R {寄存器}`：插入寄存器的内容。
    InsertRegister,
    /// `Q`：进入 Ex 模式。
    ExMode,
    /// `K`：查询光标处的关键字。
    Keyword,
    /// `U`：撤销最近对一行的全部修改。
    UndoLine,
}

/// 操作符之后强制指定的动作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Force {
    /// `v`：按字符。
    Charwise,
    /// `V`：按行。
    Linewise,
    /// `Ctrl-V`：按块。
    Blockwise,
}

/// 记录的种类，决定解析器在按键序列匹配之后的处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 动作。
    Motion(Motion),
    /// 操作符。
    Operator(Operator),
    /// 文本对象。
    TextObject(TextObject),
    /// 不带范围的命令。
    Action(Action),
    /// 寄存器前缀 `"`：其后的字符是寄存器名，之后继续输入命令。
    Register,
    /// 操作符之后强制动作类型的前缀，之后继续输入动作。
    Force(Force),
}

/// 按键序列之后的参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arg {
    /// 没有参数。
    None,
    /// 一个字符，例如 `f` 查找的字符、`m` 的标记名。
    Char,
}

/// 记录的支持状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Support {
    /// 已实现，且有录制用例核对。
    Supported,
    /// 在兼容目标之内，尚未实现或尚未核对。
    Planned,
    /// 不支持：输入后提示不支持，不执行任何操作。
    Unsupported,
}

/// 命令表中的一条记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// 按键序列（按键记法），不含次数、寄存器与参数。
    pub keys: &'static str,
    /// 记录生效的模式。
    pub modes: Modes,
    /// 种类。
    pub kind: Kind,
    /// 按键序列之后的参数。
    pub arg: Arg,
    /// 支持状态。
    pub support: Support,
}

/// 寄存器名的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterName {
    /// 可以使用的寄存器。
    Valid,
    /// 寄存器存在，但不在支持范围内（表达式寄存器 `=`、轮换文件名 `#`）。
    Unsupported,
    /// 不是寄存器名。
    Invalid,
}

/// 返回字符 `c` 作为寄存器名的类别。
pub(crate) fn register_name(c: char) -> RegisterName {
    match c {
        '"' | '-' | '_' | '+' | '*' | '/' | ':' | '.' | '%' => RegisterName::Valid,
        c if c.is_ascii_alphanumeric() => RegisterName::Valid,
        '=' | '#' => RegisterName::Unsupported,
        _ => RegisterName::Invalid,
    }
}

#[cfg(test)]
mod tests;
