//! Vim 模式的状态机：模式、按键的处理、命令的执行。
//!
//! [`Vim`] 是一个窗口的状态，[`Shared`] 是各窗口共用的状态（寄存器、上一次修改、上一次行内
//! 查找）。按键经解析器得到命令，命令在 [`Host`] 提供的文本上求值，修改与选区交给宿主应用。
//! 插入模式中的输入由宿主执行，经 [`Vim::typed`] 记录，供次数与点重复重放。

mod edit;
mod insert;
mod operate;

use winc_core::{Change, Lines, Selection, movement};

use crate::command::{Action, Mode};
use crate::host::{Host, Input, Message};
use crate::key::{Code, Key, Mods, Named};
use crate::motion::{self, Env, Find, Want, WantUpdate};
use crate::parser::{Body, Command, Movement, Outcome, Parser};
use crate::register::{Clipboard, Content, Kind, Registers, Source};
use crate::text;

/// 一次命令（次数、粘贴、重复插入）生成的文本的上限（字节）；超出时命令不执行。
///
/// 取自调参：16 MiB 远大于正常使用时生成的文本，且不会使撤销栈一次占满上限。
pub(crate) const MAX_OUTPUT: usize = 16 << 20;

/// 可视模式的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualKind {
    /// 按字符。
    Char,
    /// 按行。
    Line,
}

/// 可视模式的选区：两端字符的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Visual {
    /// 种类。
    kind: VisualKind,
    /// 起点字符的位置。
    anchor: usize,
    /// 光标所在字符的位置；可以位于行尾。
    cursor: usize,
}

/// 可视模式下的修改覆盖的范围，供点重复在光标处重建同等的选区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Extent {
    /// 种类。
    kind: VisualKind,
    /// 覆盖的行数，不小于 1。
    lines: usize,
    /// 只有一行时为覆盖的显示列数；多行时为末行上选区终点的显示列；`usize::MAX` 表示选区
    /// 延伸到末行的行尾（含换行符）。
    cells: usize,
}

/// 上一次修改的记录，供 `.` 重放。
#[derive(Debug, Clone)]
struct Repeat {
    /// 命令。
    command: Command,
    /// 命令进入插入模式之后执行的输入。
    inputs: Vec<Input>,
    /// 命令在可视模式下执行时选区覆盖的范围。
    extent: Option<Extent>,
}

/// 各窗口共用的状态。
#[derive(Debug, Default)]
pub struct Shared {
    /// 寄存器。
    pub registers: Registers,
    /// 系统剪贴板与无名寄存器的同步方式。
    pub clipboard: Clipboard,
    /// 上一次行内查找。
    last_find: Option<Find>,
    /// 上一次修改。
    repeat: Option<Repeat>,
}

/// 一个窗口的 Vim 状态。
#[derive(Debug)]
pub struct Vim {
    /// 当前的模式：普通、可视或插入。
    mode: Mode,
    /// 可视模式的选区；不在可视模式时为 `None`。
    visual: Option<Visual>,
    /// 未完成的命令。
    parser: Parser,
    /// 上下移动时保持的期望列。
    want: Want,
    /// 进行中的插入；不在插入模式时为 `None`。
    insert: Option<insert::Session>,
    /// 进行中的插入是否持有一个未结束的撤销分组。
    insert_group: bool,
    /// 上一次可视模式的选区，供 `gv` 恢复。
    last_visual: Option<Visual>,
    /// 上一次离开插入模式时的插入位置，供 `gi` 使用。
    last_insert: Option<usize>,
    /// 最近一次交给宿主的选区；宿主的选区与之不同时说明选区被其他操作改动过。
    shown: Option<Selection>,
    /// 是否正在重放上一次修改；重放期间不更新记录。
    replaying: bool,
}

impl Default for Vim {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            visual: None,
            parser: Parser::default(),
            want: Want::Cell(0),
            insert: None,
            insert_group: false,
            last_visual: None,
            last_insert: None,
            shown: None,
            replaying: false,
        }
    }
}

impl Vim {
    /// 返回当前的模式：[`Mode::Normal`]、[`Mode::Visual`] 或 [`Mode::Insert`]。
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// 返回可视模式的种类；不在可视模式时返回 `None`。
    pub fn visual_kind(&self) -> Option<VisualKind> {
        self.visual.map(|visual| visual.kind)
    }

    /// 是否已输入操作符、正在等待动作。
    pub fn is_operator_pending(&self) -> bool {
        self.parser.is_operator_pending()
    }

    /// 返回未完成命令已输入的按键，用于显示。
    pub fn pending(&self) -> &[Key] {
        self.parser.pending()
    }

    /// 返回光标所在字符的位置：可视模式下是选区中光标一端的字符，其余模式下是选区的端点。
    pub fn caret(&self, host: &impl Host) -> usize {
        match self.visual {
            Some(visual) => visual.cursor.min(host.text().end_pos()),
            None => host.selection().head,
        }
    }

    /// 处理按键 `key`，返回是否已由 Vim 层处理；返回假时按键交给宿主的其他部分。
    ///
    /// 普通与可视模式下，没有定义的可见字符同样算作已处理，不会成为输入的文字。
    pub fn key(&mut self, shared: &mut Shared, host: &mut impl Host, key: Key) -> bool {
        self.sync(host);
        if self.mode == Mode::Insert {
            return match self.parser.feed(Mode::Insert, key) {
                Outcome::Command(_) => {
                    self.leave_insert(shared, host);
                    true
                }
                Outcome::Pending | Outcome::Cancelled | Outcome::Invalid => true,
                Outcome::Unsupported => {
                    host.message(Message::Unsupported);
                    true
                }
                Outcome::Unbound => false,
            };
        }
        match self.parser.feed(self.mode, key) {
            Outcome::Command(command) => self.run(shared, host, command),
            Outcome::Unsupported => host.message(Message::Unsupported),
            Outcome::Pending | Outcome::Cancelled | Outcome::Invalid => {}
            Outcome::Unbound => return swallowed(key),
        }
        true
    }

    /// 丢弃未完成的命令；焦点离开编辑区时调用。
    pub fn cancel_pending(&mut self) {
        self.parser.reset();
    }

    /// 回到普通模式并结束未完成的撤销分组；切换文档之前对原文档调用。
    pub fn reset(&mut self, host: &mut impl Host) {
        self.parser.reset();
        if self.insert_group {
            host.end_group();
            self.insert_group = false;
        }
        self.insert = None;
        self.mode = Mode::Normal;
        // 可视模式的选区收回为光标，否则下一次同步会重新进入可视模式。
        match self.visual.take() {
            Some(visual) => self.settle(host, visual.cursor),
            None => self.shown = None,
        }
    }

    /// 回到普通模式并丢弃全部状态，不访问宿主；原文档已经关闭时调用。
    pub fn detach(&mut self) {
        self.parser.reset();
        self.insert = None;
        self.insert_group = false;
        self.visual = None;
        self.mode = Mode::Normal;
        self.shown = None;
    }

    /// 使 Vim 层的状态与宿主的选区一致；选区被 Vim 层之外的操作（鼠标、应用的命令）改动之后
    /// 调用。
    ///
    /// 普通模式下选区非空时进入可视模式，可视模式下选区变空时回到普通模式；普通模式的光标
    /// 收拢到字符上。插入模式不受影响。
    pub fn sync(&mut self, host: &mut impl Host) {
        let selection = host.selection();
        if self.mode == Mode::Insert || self.shown == Some(selection) {
            return;
        }
        let text = host.text();
        let end = text.end_pos();
        let (anchor, head) = (selection.anchor.min(end), selection.head.min(end));
        if anchor == head {
            self.visual = None;
            self.mode = Mode::Normal;
            self.settle(host, head);
            return;
        }
        // 选区是字符之间的位置，可视模式记录两端的字符。
        let (anchor, cursor) = if head > anchor {
            (anchor, movement::prev_grapheme(text, head))
        } else {
            (movement::prev_grapheme(text, anchor), head)
        };
        self.visual = Some(Visual {
            kind: VisualKind::Char,
            anchor,
            cursor,
        });
        self.mode = Mode::Visual;
        self.want = Want::Cell(text::want_cell(text, cursor, host.tab_width()));
        self.shown = Some(selection);
    }

    /// 执行一条命令；命令的全部修改属于一个撤销单元，进入插入模式时延续到离开为止。
    fn run(&mut self, shared: &mut Shared, host: &mut impl Host, command: Command) {
        host.begin_group();
        self.execute(shared, host, command);
        if self.mode == Mode::Insert {
            self.insert_group = true;
        } else {
            host.end_group();
        }
    }

    /// 执行命令 `command`，不处理撤销分组。
    fn execute(&mut self, shared: &mut Shared, host: &mut impl Host, command: Command) {
        match command.body {
            Body::Motion(movement) => self.motion(shared, host, movement, command.count),
            Body::Operator {
                operator,
                target,
                force,
            } => self.operate(shared, host, command, operator, target, force),
            Body::Action { action, arg } => self.act(shared, host, command, action, arg),
            // 文本对象尚未实现，解析器不会给出这类命令。
            Body::Object(_) => host.message(Message::Unsupported),
        }
    }

    /// 执行不带操作符的动作：移动光标，可视模式下延伸选区。
    fn motion(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        movement: Movement,
        count: Option<u32>,
    ) {
        let env = Env {
            host: &*host,
            from: self.caret(host),
            want: self.want,
            count,
            operator: None,
            visual: self.visual.is_some(),
        };
        if let Some(moved) = motion::evaluate(&env, movement, &mut shared.last_find) {
            self.goto(host, moved.pos, moved.want);
        }
    }

    /// 把光标移到 `pos` 并按 `want` 更新期望列；可视模式下移动选区的光标一端。
    fn goto(&mut self, host: &mut impl Host, pos: usize, want: WantUpdate) {
        let text = host.text();
        let pos = match &mut self.visual {
            Some(visual) => {
                let line = text::line_of(text, pos);
                let pos = pos.clamp(text.line_start(line), text.line_end(line));
                visual.cursor = movement::snap_to_grapheme(text, pos);
                visual.cursor
            }
            None => text::on_char(text, pos),
        };
        match want {
            WantUpdate::Keep => {}
            WantUpdate::Set(want) => self.want = want,
            WantUpdate::Recompute => {
                self.want = Want::Cell(text::want_cell(text, pos, host.tab_width()));
            }
        }
        self.show(host, pos);
    }

    /// 把普通模式的光标放到 `pos` 所在的字符上，并重新计算期望列。
    fn settle(&mut self, host: &mut impl Host, pos: usize) {
        self.goto(host, pos, WantUpdate::Recompute);
    }

    /// 把选区交给宿主：可视模式下是包含两端字符的选区，否则是位于 `pos` 的光标。
    fn show(&mut self, host: &mut impl Host, pos: usize) {
        let text = host.text();
        let selection = match self.visual {
            None => Selection::point(pos),
            Some(visual) => {
                let (low, high) = (
                    visual.anchor.min(visual.cursor),
                    visual.anchor.max(visual.cursor),
                );
                let (from, to) = match visual.kind {
                    VisualKind::Char => (low, text::after(text, high)),
                    VisualKind::Line => {
                        let first = text::line_of(text, low);
                        let last = text::line_of(text, high);
                        (text.line_start(first), text.line_span(last).end)
                    }
                };
                if visual.cursor >= visual.anchor {
                    Selection::new(from, to)
                } else {
                    Selection::new(to, from)
                }
            }
        };
        host.set_selection(selection);
        self.shown = Some(selection);
    }

    /// 应用一组修改；`at` 是修改之前光标所在的位置，`first_line` 是第一处变化所在的行。
    ///
    /// 撤销之后的光标按以下规则填写：`at` 在第一处变化所在的行或其上一行时回到 `at`，否则位于
    /// 该行的第一个非空白字符。返回宿主是否接受了修改。
    fn edit(
        &mut self,
        host: &mut impl Host,
        changes: Vec<Change>,
        at: usize,
        first_line: usize,
    ) -> bool {
        if changes.is_empty() {
            return true;
        }
        let text = host.text();
        let at_line = text::line_of(text, at);
        let line = if at_line + 1 == first_line {
            at_line
        } else {
            first_line
        };
        let before = if line > text::last_line(text) {
            text.line_start(text::last_line(text))
        } else if line == at_line {
            at
        } else {
            text::first_non_blank(text, line)
        };
        let applied = host.apply(changes, before, before);
        if !applied {
            host.message(Message::TooLarge);
        }
        self.shown = None;
        applied
    }

    /// 文档只读时提示并返回真。
    fn blocked(host: &mut impl Host) -> bool {
        let blocked = host.read_only();
        if blocked {
            host.message(Message::ReadOnly);
        }
        blocked
    }

    /// 离开可视模式，记下选区供 `gv` 恢复；光标的位置由调用方设置。
    fn end_visual(&mut self) {
        if let Some(visual) = self.visual.take() {
            self.last_visual = Some(visual);
        }
        self.mode = Mode::Normal;
    }

    /// 读取寄存器 `name` 的内容；`None` 表示未指定。
    fn read_register(
        shared: &mut Shared,
        host: &mut impl Host,
        name: Option<char>,
    ) -> Option<Content> {
        let read_clipboard = |shared: &Shared, host: &mut dyn FnMut() -> Option<String>| {
            host().map(|text| shared.registers.read_clipboard(&text))
        };
        match name {
            Some('_') => None,
            Some('+' | '*') => read_clipboard(shared, &mut || host.clipboard()),
            Some('%') => host.file_name().map(|text| Content {
                kind: Kind::Charwise,
                text,
            }),
            None | Some('"') if shared.clipboard == Clipboard::Always => {
                read_clipboard(shared, &mut || host.clipboard())
                    .or_else(|| shared.registers.get(None).cloned())
            }
            name => shared.registers.get(name).cloned(),
        }
    }

    /// 把 `content` 写入寄存器 `name`；寄存器只读时提示并返回假。
    fn write_register(
        shared: &mut Shared,
        host: &mut impl Host,
        name: Option<char>,
        content: Content,
        source: Source,
    ) -> bool {
        match name {
            Some('_') => {}
            Some('+' | '*') => {
                let text = shared.registers.write_clipboard(&content);
                host.set_clipboard(&text);
            }
            Some(name @ ('.' | ':' | '/' | '%')) => {
                host.message(Message::ReadOnlyRegister(name));
                return false;
            }
            name => {
                let unnamed = name.is_none_or(|name| name == '"');
                let synced = match shared.clipboard {
                    Clipboard::Register => false,
                    Clipboard::Yank => source == Source::Yank,
                    Clipboard::Always => true,
                };
                if unnamed && synced {
                    let text = shared.registers.write_clipboard(&content);
                    host.set_clipboard(&text);
                }
                shared.registers.write(name, content, source);
            }
        }
        true
    }

    /// 记录上一次修改；重放期间不记录。
    fn remember(&mut self, shared: &mut Shared, command: Command, extent: Option<Extent>) {
        if !self.replaying {
            shared.repeat = Some(Repeat {
                command,
                inputs: Vec::new(),
                extent,
            });
        }
    }

    /// 执行不带范围的命令。
    fn act(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        command: Command,
        action: Action,
        arg: Option<char>,
    ) {
        match action {
            Action::Insert
            | Action::Append
            | Action::InsertFirstNonBlank
            | Action::InsertLineStart
            | Action::AppendLineEnd
            | Action::InsertAtLast
            | Action::OpenBelow
            | Action::OpenAbove => self.start_insert(host, command, action),
            Action::ReplaceChar => self.replace_char(shared, host, command, arg),
            Action::ToggleCaseChar => self.toggle_case_char(shared, host, command),
            Action::PasteAfter
            | Action::PasteBefore
            | Action::PasteAfterMove
            | Action::PasteBeforeMove
            | Action::PasteIndented => self.paste(shared, host, command, action),
            Action::JoinLines | Action::JoinLinesRaw => {
                self.join_lines(shared, host, command, action == Action::JoinLines);
            }
            Action::Undo | Action::Redo => {
                let times = command.count.unwrap_or(1);
                // 重做之后光标回到修改之前所在的行与列，而不是修改之后的位置；该行已不存在时
                // 放在最后一行的行首。
                let mut origin = None;
                for _ in 0..times {
                    if action == Action::Undo {
                        if !host.undo() {
                            break;
                        }
                    } else {
                        let text = host.text();
                        let before = host.redo_origin().map(|pos| text.pos_to_line_col(pos));
                        if !host.redo() {
                            break;
                        }
                        origin = before;
                    }
                }
                self.shown = None;
                let text = host.text();
                let pos = match origin {
                    Some((line, _)) if line > text::last_line(text) => {
                        text.line_start(text::last_line(text))
                    }
                    Some((line, col)) => text.line_col_to_pos(line, col),
                    None => host.selection().head.min(text.end_pos()),
                };
                self.settle(host, pos);
            }
            Action::Repeat => self.repeat(shared, host, command.count),
            Action::Visual | Action::VisualLine => {
                let kind = if action == Action::Visual {
                    VisualKind::Char
                } else {
                    VisualKind::Line
                };
                self.toggle_visual(host, kind);
            }
            Action::Reselect => self.reselect(host),
            Action::SwapAnchor => {
                if let Some(visual) = &mut self.visual {
                    std::mem::swap(&mut visual.anchor, &mut visual.cursor);
                    let cursor = visual.cursor;
                    self.goto(host, cursor, WantUpdate::Recompute);
                }
            }
            Action::Escape => {
                if let Some(visual) = self.visual {
                    self.end_visual();
                    self.settle(host, visual.cursor);
                }
            }
            Action::Later | Action::ExMode | Action::Keyword | Action::UndoLine => {
                host.message(Message::Unsupported);
            }
        }
    }

    /// 进入、切换或离开可视模式。
    fn toggle_visual(&mut self, host: &mut impl Host, kind: VisualKind) {
        let cursor = self.caret(host);
        match &mut self.visual {
            Some(visual) if visual.kind == kind => {
                self.end_visual();
                self.settle(host, cursor);
            }
            Some(visual) => {
                visual.kind = kind;
                self.show(host, cursor);
            }
            None => {
                self.visual = Some(Visual {
                    kind,
                    anchor: cursor,
                    cursor,
                });
                self.mode = Mode::Visual;
                self.show(host, cursor);
            }
        }
    }

    /// `gv`：恢复上一次的可视选区；已在可视模式时与当前的选区交换。
    fn reselect(&mut self, host: &mut impl Host) {
        let Some(mut last) = self.last_visual else {
            return;
        };
        let text = host.text();
        let clamp = |pos: usize| {
            let line = text::line_of(text, pos.min(text.end_pos()));
            pos.clamp(text.line_start(line), text.line_end(line))
        };
        last.anchor = clamp(last.anchor);
        last.cursor = clamp(last.cursor);
        if let Some(current) = self.visual {
            self.last_visual = Some(current);
        }
        self.visual = Some(last);
        self.mode = Mode::Visual;
        self.goto(host, last.cursor, WantUpdate::Recompute);
    }

    /// `.`：重放上一次修改；`count` 给出时替换原来的次数。
    fn repeat(&mut self, shared: &mut Shared, host: &mut impl Host, count: Option<u32>) {
        let Some(mut repeat) = shared.repeat.clone() else {
            return;
        };
        if count.is_some() {
            repeat.command.count = count;
        }
        // 重复从编号寄存器粘贴时改用下一个编号，依次取出更早删除的文本。
        if let (Some(digit @ '1'..='8'), Body::Action { .. }) =
            (repeat.command.register, repeat.command.body)
        {
            repeat.command.register = char::from_digit(digit.to_digit(10).unwrap_or(0) + 1, 10);
        }
        if let Some(stored) = &mut shared.repeat {
            stored.command = repeat.command;
        }
        if self.visual.is_some() {
            return;
        }
        if let Some(extent) = repeat.extent {
            self.select_extent(host, extent);
        }
        self.replaying = true;
        self.execute(shared, host, repeat.command);
        if self.mode == Mode::Insert {
            for input in &repeat.inputs {
                host.input(input);
                self.typed(input.clone());
            }
            self.leave_insert(shared, host);
        }
        self.replaying = false;
    }

    /// 在光标处建立与 `extent` 同等范围的可视选区。
    fn select_extent(&mut self, host: &mut impl Host, extent: Extent) {
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let last = (line + extent.lines - 1).min(text::last_line(text));
        let tab_width = host.tab_width();
        let end = if extent.cells == usize::MAX {
            text.line_end(last)
        } else if extent.lines == 1 {
            // 行尾算作一列：选区可以延伸到行尾，把换行符包括在内。
            let cell = text::want_cell(text, cursor, tab_width) + extent.cells - 1;
            text::pos_at_cell(text, line, cell, tab_width)
        } else {
            text::pos_at_cell(text, last, extent.cells, tab_width)
        };
        self.visual = Some(Visual {
            kind: extent.kind,
            anchor: cursor,
            cursor: end,
        });
        self.mode = Mode::Visual;
    }
}

/// 普通与可视模式下没有定义的按键是否由 Vim 层吞掉：可见字符不交给宿主，其余按键交给宿主。
fn swallowed(key: Key) -> bool {
    let plain = key.mods().without(Mods::SHIFT).is_empty();
    match key.code() {
        Code::Char(_) => plain,
        Code::Named(named) => {
            plain
                && matches!(
                    named,
                    Named::Tab | Named::Enter | Named::Backspace | Named::Delete
                )
        }
    }
}
