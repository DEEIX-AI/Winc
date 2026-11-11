//! 插入模式：进入、输入的记录、次数的重复与离开。

use winc_core::{Change, Lines, Selection};

use super::{Extent, Repeat, Shared, Vim};
use crate::command::{Action, Mode};
use crate::host::{Host, Input};
use crate::motion::WantUpdate;
use crate::parser::Command;
use crate::text;

/// 一次插入记录的输入占用的上限（字节）；超出之后的输入不再记录。
///
/// 取自调参：1 MiB 远多于一次连续输入的文字，重复插入时生成的文本另有上限。
const MAX_RECORD: usize = 1 << 20;

/// 一次进行中的插入。
#[derive(Debug)]
pub(super) struct Session {
    /// 引起这次插入的命令，离开时连同输入记为上一次修改。
    command: Command,
    /// 离开时输入重复的次数，不小于 1。
    count: usize,
    /// 已执行的输入。
    inputs: Vec<Input>,
    /// 已记录的输入占用的字节数。
    bytes: usize,
    /// 由 `o`（真）或 `O`（假）进入时，每次重复之前先新开一行。
    open: Option<bool>,
    /// 命令在可视模式下执行时选区覆盖的范围。
    pub(super) extent: Option<Extent>,
}

/// 返回输入 `input` 占用的字节数。
fn size(input: &Input) -> usize {
    match input {
        Input::Text(text) | Input::Paste(text) => text.len().max(1),
        _ => 1,
    }
}

impl Vim {
    /// 执行进入插入模式的命令 `action`。
    pub(super) fn start_insert(&mut self, host: &mut impl Host, command: Command, action: Action) {
        if Self::blocked(host) {
            return;
        }
        let text = host.text();
        let cursor = self.caret(host);
        let line = text::line_of(text, cursor);
        let (_, last) = host
            .fold(line)
            .filter(|_| host.has_folds())
            .unwrap_or((line, line));
        let pos = match action {
            Action::Append => text::next_in_line(text, cursor).unwrap_or(cursor),
            Action::InsertFirstNonBlank => text.line_start(line) + text::indent_chars(text, line),
            Action::InsertLineStart => text.line_start(line),
            Action::AppendLineEnd => text.line_end(line),
            Action::InsertAtLast => match self.last_insert {
                Some(pos) => {
                    let pos = pos.min(text.end_pos());
                    let line = text::line_of(text, pos);
                    pos.clamp(text.line_start(line), text.line_end(line))
                }
                None => cursor,
            },
            Action::OpenBelow => {
                self.open_line(host, last, true);
                self.begin_insert(host, command, host.selection().head, Some(true));
                return;
            }
            Action::OpenAbove => {
                self.open_line(host, line, false);
                self.begin_insert(host, command, host.selection().head, Some(false));
                return;
            }
            _ => cursor,
        };
        self.begin_insert(host, command, pos, None);
    }

    /// 在第 `line` 行的下方（`below` 为真）或上方新开一行，光标放在新行上。
    fn open_line(&mut self, host: &mut impl Host, line: usize, below: bool) {
        let text = host.text();
        if below {
            host.open_line(text.line_end(line));
            self.shown = None;
            return;
        }
        let cursor = self.caret(host);
        let start = text.line_start(line);
        let indent = if host.keeps_indent() {
            text.substring(start, start + text::indent_chars(text, line))
        } else {
            String::new()
        };
        let pos = start + indent.chars().count();
        let change = Change {
            start,
            removed: String::new(),
            inserted: format!("{indent}{}", host.line_ending()),
        };
        self.edit(host, vec![change], cursor, line);
        host.set_selection(Selection::point(pos));
    }

    /// 进入插入模式，光标位于 `pos`；`open` 的含义见 [`Session`]。
    pub(super) fn begin_insert(
        &mut self,
        host: &mut impl Host,
        command: Command,
        pos: usize,
        open: Option<bool>,
    ) {
        // 修改操作符的次数属于动作，只有直接进入插入模式的命令才重复输入。
        let direct = matches!(command.body, crate::parser::Body::Action { .. });
        let count = match command.count {
            Some(count) if direct => usize::try_from(count).unwrap_or(usize::MAX),
            _ => 1,
        };
        self.visual = None;
        self.mode = Mode::Insert;
        self.parser.reset();
        self.insert = Some(Session {
            command,
            count,
            inputs: Vec::new(),
            bytes: 0,
            open,
            extent: None,
        });
        let selection = Selection::point(pos);
        host.set_selection(selection);
        self.shown = Some(selection);
    }

    /// 记录插入模式中宿主已经执行的一次输入。
    pub fn typed(&mut self, input: Input) {
        if let Some(session) = &mut self.insert {
            let size = size(&input);
            if session.bytes + size <= MAX_RECORD {
                session.bytes += size;
                session.inputs.push(input);
            }
        }
    }

    /// 插入模式中光标被移动（方向键、鼠标）或执行了输入之外的编辑时调用。
    ///
    /// 此前的输入结束一个撤销单元；之后的输入另起一个，点重复只重放之后的输入。
    pub fn insert_moved(&mut self, host: &mut impl Host) {
        let Some(session) = &mut self.insert else {
            return;
        };
        session.inputs.clear();
        session.bytes = 0;
        session.count = 1;
        session.open = None;
        session.extent = None;
        session.command = Command {
            register: None,
            count: None,
            body: crate::parser::Body::Action {
                action: Action::Insert,
                arg: None,
            },
        };
        if self.insert_group {
            host.end_group();
            host.begin_group();
        }
    }

    /// 离开插入模式：按次数重复输入，光标左移一个字符，并把这次插入记为上一次修改。
    pub(super) fn leave_insert(&mut self, shared: &mut Shared, host: &mut impl Host) {
        let Some(session) = self.insert.take() else {
            return;
        };
        self.parser.reset();

        // 次数大于 1 时重复输入；生成的文本超过上限时只执行一次。新开的行按一个字节计。
        let repeats = session.count - 1;
        let each = session.bytes + usize::from(session.open.is_some());
        if repeats > 0 && each > 0 && each.saturating_mul(repeats) <= super::MAX_OUTPUT {
            for _ in 0..repeats {
                // 重复时新行都开在刚输入的一行的下方。
                if session.open.is_some() {
                    let line = text::line_of(host.text(), host.selection().head);
                    self.open_line(host, line, true);
                }
                for input in &session.inputs {
                    host.input(input);
                }
            }
        }

        let text = host.text();
        let pos = host.selection().head.min(text.end_pos());
        self.last_insert = Some(pos);
        self.mode = Mode::Normal;
        let left = text::prev_in_line(text, pos).unwrap_or(pos);
        self.goto(host, left, WantUpdate::Recompute);
        if self.insert_group {
            host.end_group();
            self.insert_group = false;
        }

        let typed: String = (session.inputs.iter())
            .map(|input| match input {
                Input::Text(text) | Input::Paste(text) => text.as_str(),
                Input::Newline => "\n",
                _ => "",
            })
            .collect();
        shared.registers.set_special('.', typed);
        if !self.replaying {
            shared.repeat = Some(Repeat {
                command: session.command,
                inputs: session.inputs,
                extent: session.extent,
            });
        }
    }
}
