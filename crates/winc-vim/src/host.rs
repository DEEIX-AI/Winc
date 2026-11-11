//! 宿主接口：Vim 层读取文本与视图、应用修改所依赖的 trait。
//!
//! Vim 层自身不保存文本。命令求值得到的修改以 [`Change`] 的形式交给宿主应用；插入模式中的
//! 输入（[`Input`]）由宿主按自己的规则执行（自动缩进、括号补全），Vim 层只记录并重放。

use std::ops::Range;

use winc_core::{Change, Lines, Selection};

/// 插入模式中的一次输入，由宿主执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// 键入文字。
    Text(String),
    /// 粘贴文字。
    Paste(String),
    /// 换行。
    Newline,
    /// 向前删除一个字符；`word` 为真时删除一个词。
    Backspace {
        /// 是否按词删除。
        word: bool,
    },
    /// 向后删除一个字符；`word` 为真时删除一个词。
    Delete {
        /// 是否按词删除。
        word: bool,
    },
    /// 删除光标到行首之间的文本。
    DeleteToLineStart,
    /// 插入一级缩进。
    Indent,
    /// 去掉一级缩进。
    Outdent,
    /// 光标所在的行增加一级缩进。
    IndentLine,
}

/// 需要显示给用户的提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// 输入的命令不支持或尚未实现。
    Unsupported,
    /// 文档只读，修改类命令没有执行。
    ReadOnly,
    /// 命令涉及的文本超过了可以修改的大小，或次数使结果超过了上限。
    TooLarge,
    /// 寄存器中没有内容；值为寄存器名。
    EmptyRegister(char),
    /// 寄存器不能写入；值为寄存器名。
    ReadOnlyRegister(char),
    /// 没有找到查找的模式。
    NotFound,
    /// 模式中有无法转换的写法；值为该写法在模式中的字符偏移。
    BadPattern(usize),
    /// 还没有查找过，无法重复上一次查找。
    NoSearch,
    /// 标记没有设置；值为标记名。
    NoMark(char),
}

/// 滚动之后一行在窗口中的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// 窗口顶部。
    Top,
    /// 窗口中间。
    Center,
    /// 窗口底部。
    Bottom,
}

/// 滚动窗口的请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    /// 纵向滚动若干行，正数使窗口显示更靠后的文本。
    Lines(isize),
    /// 纵向滚动到第 `line` 行位于窗口的 `place`。
    Reveal {
        /// 行号。
        line: usize,
        /// 该行在窗口中的位置。
        place: Place,
    },
    /// 横向滚动若干显示列，正数使窗口显示更靠右的文本。
    Columns(isize),
}

/// 对折叠的操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    /// 展开一层。
    Open,
    /// 收起一层。
    Close,
    /// 切换一层。
    Toggle,
    /// 展开各层。
    OpenTree,
    /// 收起各层。
    CloseTree,
    /// 切换各层。
    ToggleTree,
    /// 展开文档中的全部折叠。
    OpenAll,
    /// 收起文档中的全部折叠。
    CloseAll,
    /// 展开到该行可见。
    Reveal,
}

/// 转换之后的查找模式，由宿主编译并执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// 正则表达式，语法为 `regex` 与 `fancy-regex` 共同接受的写法；`^`、`$` 按行匹配。
    pub regex: String,
    /// 是否忽略大小写。
    pub ignore_case: bool,
    /// 模式是否以字面的字符结尾；为真时，终点之后紧跟组合字符的匹配不算。
    pub ends_literal: bool,
}

/// 缩进的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indentation {
    /// 一级缩进的宽度（显示列），不小于 1。
    pub width: usize,
    /// 是否用制表符组成缩进；为假时全部使用空格。
    pub tabs: bool,
}

/// 宿主：提供文本、选区与视图，并应用 Vim 层给出的修改。
///
/// 位置都是 `char` 偏移，行号从 0 起。
pub trait Host {
    /// 文本的类型。
    type Text: Lines;

    /// 返回当前的文本。
    fn text(&self) -> &Self::Text;

    /// 返回主选区。
    fn selection(&self) -> Selection;

    /// 设置主选区，不修改文本；其余选区不变。
    fn set_selection(&mut self, selection: Selection);

    /// 依次应用 `changes`，并把光标设在 `after`；返回是否已应用。主选区之外的选区被收回。
    ///
    /// 每处修改的起点是应用之前各处修改后的位置。`before` 是撤销这些修改之后光标所在的位置。
    /// 同一个分组内的多次调用属于同一个撤销单元。
    fn apply(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool;

    /// 在主选区处执行插入模式的一次输入。
    fn input(&mut self, input: &Input);

    /// 在位置 `at`（某一行的行尾）换行，光标放在新行上；新行的缩进由宿主决定。
    ///
    /// 撤销之后光标回到调用之前的位置。
    fn open_line(&mut self, at: usize);

    /// 撤销一个撤销单元，并把选区恢复到修改之前；没有可撤销的内容时返回假。
    fn undo(&mut self) -> bool;

    /// 重做一个撤销单元，并把选区恢复到修改之后；没有可重做的内容时返回假。
    fn redo(&mut self) -> bool;

    /// 返回下一次重做的撤销单元执行之前光标所在的位置；没有可重做的内容时返回 `None`。
    fn redo_origin(&self) -> Option<usize>;

    /// 开始一个撤销分组：到对应的 [`Host::end_group`] 为止的修改合并成一个撤销单元。
    fn begin_group(&mut self);

    /// 结束最近一次 [`Host::begin_group`] 开始的分组。
    fn end_group(&mut self);

    /// 是否不能修改文本。
    fn read_only(&self) -> bool {
        false
    }

    /// 是否可以读取并修改位置区间 `from..to` 内的文本；区间过大时返回假。
    fn fits(&self, from: usize, to: usize) -> bool {
        let _ = (from, to);
        true
    }

    /// 返回制表符的宽度（显示列），不小于 1。
    fn tab_width(&self) -> usize;

    /// 返回缩进的方式，供增减缩进使用。
    fn indentation(&self) -> Indentation;

    /// 是否在新开的行和重新输入的行上保留原有的缩进。
    fn keeps_indent(&self) -> bool {
        false
    }

    /// 返回新插入的换行所用的换行符。
    fn line_ending(&self) -> &str {
        "\n"
    }

    /// 返回窗口中完整显示的文本行的行号范围。
    fn visible_lines(&self) -> Range<usize>;

    /// 返回窗口可以容纳的显示行数，供按半屏、整屏滚动使用；默认取完整显示的文本行数。
    fn window_height(&self) -> usize {
        let visible = self.visible_lines();
        visible.end.saturating_sub(visible.start).max(1)
    }

    /// 返回文字区一行可以容纳的显示列数。
    fn text_width(&self) -> usize {
        80
    }

    /// 返回 `pos` 所在显示行的位置范围：起点是该显示行的第一个字符，终点在最后一个字符之后。
    ///
    /// 不换行时为整行，即默认实现。
    fn display_row(&self, pos: usize) -> Range<usize> {
        let line = self.text().line_of(pos);
        self.text().line_start(line)..self.text().line_end(line)
    }

    /// 返回从 `pos` 上下移动 `delta` 个显示行、并对齐到显示列 `cell` 之后的位置。
    ///
    /// 返回 `None` 表示按文本行移动，即默认实现。
    fn display_vertical(&self, pos: usize, delta: isize, cell: usize) -> Option<usize> {
        let _ = (pos, delta, cell);
        None
    }

    /// 返回包含第 `line` 行的已折叠区域的首行与末行；该行不在折叠中时返回 `None`。
    fn fold(&self, line: usize) -> Option<(usize, usize)> {
        let _ = line;
        None
    }

    /// 是否存在已折叠的区域；为假时 [`Host::fold`] 不会被调用。
    fn has_folds(&self) -> bool {
        false
    }

    /// 返回系统剪贴板中的文本；没有文本或无法读取时返回 `None`。
    fn clipboard(&mut self) -> Option<String> {
        None
    }

    /// 把 `text` 写入系统剪贴板。
    fn set_clipboard(&mut self, text: &str) {
        let _ = text;
    }

    /// 切换第 `first` 至 `last` 行的行注释；返回是否已执行。
    fn toggle_comment(&mut self, first: usize, last: usize) -> bool {
        let _ = (first, last);
        false
    }

    /// 格式化第 `first` 至 `last` 行；返回是否已执行。
    fn format_lines(&mut self, first: usize, last: usize) -> bool {
        let _ = (first, last);
        false
    }

    /// 返回文档的文件名，供 `"%` 寄存器使用。
    fn file_name(&self) -> Option<String> {
        None
    }

    /// 返回全部选区，按位置排序；只有主选区时返回一项。
    fn selections(&self) -> Vec<Selection> {
        vec![self.selection()]
    }

    /// 把选区设为 `selections`，主选区是其中的第 `primary` 项；不修改文本。
    ///
    /// 默认实现只设置主选区。
    fn set_selections(&mut self, selections: Vec<Selection>, primary: usize) {
        if let Some(selection) = selections.get(primary) {
            self.set_selection(*selection);
        }
    }

    /// 返回起点不小于 `from`（`forward` 为真）或小于 `from` 的最近一处匹配的位置区间。
    ///
    /// 匹配互不重叠，每行从行首起依次确定；找不到、模式无法编译时返回 `None`。同时把该模式
    /// 设为宿主当前高亮的查找内容。
    fn find(&mut self, query: &Query, from: usize, forward: bool) -> Option<(usize, usize)> {
        let _ = (query, from, forward);
        None
    }

    /// 返回标记 `name` 的位置；标记随编辑移动，没有设置时返回 `None`。
    fn mark(&self, name: char) -> Option<usize> {
        let _ = name;
        None
    }

    /// 把标记 `name` 设在位置 `pos`。
    fn set_mark(&mut self, name: char, pos: usize) {
        let _ = (name, pos);
    }

    /// 返回修改列表中的位置，从旧到新排列；位置随编辑移动。
    fn changes(&self) -> Vec<usize> {
        Vec::new()
    }

    /// 把位置 `pos` 记入修改列表。
    fn record_change(&mut self, pos: usize) {
        let _ = pos;
    }

    /// 把位置 `pos` 记入跳转历史。
    fn record_jump(&mut self, pos: usize) {
        let _ = pos;
    }

    /// 在跳转历史中后退（`back` 为真）或前进 `count` 步，并把光标移到该处；返回是否移动。
    ///
    /// 目标可以在另一个文档中，此时宿主切换文档。
    fn jump(&mut self, back: bool, count: usize) -> bool {
        let _ = (back, count);
        false
    }

    /// 滚动窗口，返回窗口是否移动；光标不动。
    fn scroll(&mut self, scroll: Scroll) -> bool {
        let _ = scroll;
        false
    }

    /// 返回窗口中显示的显示列范围；自动换行时没有横向滚动，返回 `None`。
    fn visible_cells(&self) -> Option<Range<usize>> {
        None
    }

    /// 返回光标与窗口上下边缘之间保留的行数。
    fn scroll_margin(&self) -> usize {
        0
    }

    /// 对第 `line` 行所在的折叠执行 `fold`；返回是否有变化。
    fn fold_at(&mut self, line: usize, fold: Fold) -> bool {
        let _ = (line, fold);
        false
    }

    /// 返回第 `line` 行之后下一个可折叠区域的首行（`forward` 为真），或之前上一个可折叠区域
    /// 的末行。
    fn fold_edge(&mut self, line: usize, forward: bool) -> Option<usize> {
        let _ = (line, forward);
        None
    }

    /// 向用户显示提示 `message`。
    fn message(&mut self, message: Message) {
        let _ = message;
    }
}
