//! 测试用的宿主：以 `Rope` 保存文本，修改经撤销历史记录；插入模式的输入按原样执行，
//! 没有自动缩进与括号补全。

#![allow(dead_code, reason = "各测试文件只用到其中一部分")]

use std::ops::Range;

use ropey::Rope;
use winc_core::edit::{self, EditKind};
use winc_core::{Change, History, Lines, Selection, Transaction, movement};
use winc_vim::{Code, Host, Indentation, Input, Key, Message, Mode, Named, Shared, Vim, notation};

/// 测试用的宿主。
pub struct TestHost {
    /// 文本。
    pub rope: Rope,
    /// 主选区。
    pub selection: Selection,
    /// 撤销历史。
    pub history: History,
    /// 系统剪贴板的替身。
    pub clipboard: Option<String>,
    /// 收到的提示。
    pub messages: Vec<Message>,
    /// 窗口中显示的行；`None` 表示从光标所在的行起的 23 行，与无界面录制时的窗口相同。
    pub visible: Option<Range<usize>>,
    /// 是否只读。
    pub read_only: bool,
    /// 已折叠的区域：首行与末行。
    pub folds: Vec<(usize, usize)>,
    /// 新开的行是否保留缩进。
    pub keeps_indent: bool,
    /// 缩进的方式。
    pub indentation: Indentation,
    /// 未结束的撤销分组的层数。
    pub groups: usize,
    /// 收到的切换注释的请求：首行与末行。
    pub commented: Vec<(usize, usize)>,
    /// 自动换行的宽度（字符）；`None` 表示不换行。
    pub wrap: Option<usize>,
}

impl TestHost {
    /// 创建文本为 `text`、光标在开头的宿主；缩进方式与 `vim` 不加载配置时相同。
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            selection: Selection::point(0),
            history: History::default(),
            clipboard: None,
            messages: Vec::new(),
            visible: None,
            read_only: false,
            folds: Vec::new(),
            keeps_indent: false,
            indentation: Indentation {
                width: 8,
                tabs: true,
            },
            groups: 0,
            commented: Vec::new(),
            wrap: None,
        }
    }

    /// 应用事务并记入撤销历史。
    fn commit(&mut self, transaction: Transaction) {
        transaction.apply(&mut self.rope);
        self.selection = transaction.after.clamped(self.rope.len_chars());
        if !transaction.changes.is_empty() {
            self.history.push(transaction);
        }
    }

    /// 返回文本。
    pub fn text(&self) -> String {
        self.rope.to_string()
    }
}

impl Host for TestHost {
    type Text = Rope;

    fn text(&self) -> &Rope {
        &self.rope
    }

    fn selection(&self) -> Selection {
        self.selection
    }

    fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
    }

    fn apply(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool {
        let transaction = Transaction::new(
            changes,
            Selection::point(before),
            Selection::point(after),
            EditKind::Other,
        );
        self.commit(transaction);
        true
    }

    fn input(&mut self, input: &Input) {
        let selection = self.selection;
        let head = selection.head;
        let delete = |rope: &Rope, from: usize, to: usize| {
            edit::replace_selection(rope, Selection::new(from, to), "", EditKind::Other)
        };
        let transaction = match input {
            Input::Text(text) | Input::Paste(text) => {
                edit::replace_selection(&self.rope, selection, text, EditKind::Typing)
            }
            Input::Newline => edit::replace_selection(&self.rope, selection, "\n", EditKind::Other),
            Input::Indent => edit::replace_selection(&self.rope, selection, "\t", EditKind::Other),
            Input::Backspace { word: false } => {
                delete(&self.rope, movement::prev_grapheme(&self.rope, head), head)
            }
            Input::Backspace { word: true } => {
                delete(&self.rope, movement::prev_word(&self.rope, head), head)
            }
            Input::Delete { word: false } => {
                delete(&self.rope, head, movement::next_grapheme(&self.rope, head))
            }
            Input::Delete { word: true } => {
                delete(&self.rope, head, movement::next_word(&self.rope, head))
            }
            Input::DeleteToLineStart => {
                let start = self.rope.line_start(self.rope.line_of(head));
                delete(&self.rope, start, head)
            }
            Input::Outdent => return,
        };
        self.commit(transaction);
    }

    fn open_line(&mut self, at: usize) {
        let mut transaction =
            edit::replace_selection(&self.rope, Selection::point(at), "\n", EditKind::Other);
        transaction.before = self.selection;
        self.commit(transaction);
    }

    fn undo(&mut self) -> bool {
        match self.history.undo(&mut self.rope) {
            Some(restored) => {
                self.selection = restored.selection.clamped(self.rope.len_chars());
                true
            }
            None => false,
        }
    }

    fn redo(&mut self) -> bool {
        match self.history.redo(&mut self.rope) {
            Some(restored) => {
                self.selection = restored.selection.clamped(self.rope.len_chars());
                true
            }
            None => false,
        }
    }

    fn redo_origin(&self) -> Option<usize> {
        self.history.redo_origin().map(|selection| selection.head)
    }

    fn begin_group(&mut self) {
        self.groups += 1;
        self.history.begin_group();
    }

    fn end_group(&mut self) {
        self.groups = self.groups.checked_sub(1).expect("分组的开始与结束成对");
        self.history.end_group();
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn tab_width(&self) -> usize {
        8
    }

    fn indentation(&self) -> Indentation {
        self.indentation
    }

    fn keeps_indent(&self) -> bool {
        self.keeps_indent
    }

    fn visible_lines(&self) -> Range<usize> {
        self.visible.clone().unwrap_or_else(|| {
            let top = self.rope.line_of(self.selection.head);
            top..top + 23
        })
    }

    fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }

    fn fold(&self, line: usize) -> Option<(usize, usize)> {
        (self.folds.iter())
            .copied()
            .find(|(first, last)| (*first..=*last).contains(&line))
    }

    fn clipboard(&mut self) -> Option<String> {
        self.clipboard.clone()
    }

    fn set_clipboard(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }

    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }

    fn toggle_comment(&mut self, first: usize, last: usize) -> bool {
        self.commented.push((first, last));
        true
    }

    fn text_width(&self) -> usize {
        self.wrap.unwrap_or(80)
    }

    fn display_row(&self, pos: usize) -> Range<usize> {
        let (line, col) = self.rope.pos_to_line_col(pos);
        let start = self.rope.line_start(line);
        let end = self.rope.line_end(line);
        match self.wrap {
            // 每 `width` 个字符一个显示行；位于行尾时属于最后一个显示行。
            Some(width) => {
                let row = col.min(self.rope.line_len(line).saturating_sub(1)) / width;
                (start + row * width)..(start + (row + 1) * width).min(end)
            }
            None => start..end,
        }
    }

    fn display_vertical(&self, pos: usize, delta: isize, cell: usize) -> Option<usize> {
        let width = self.wrap?;
        // 把各行的显示行依次排开，按显示行的序号移动。
        let rows = |line: usize| self.rope.line_len(line).div_ceil(width).max(1);
        let (mut line, col) = self.rope.pos_to_line_col(pos);
        let mut row = col.min(self.rope.line_len(line).saturating_sub(1)) / width;
        for _ in 0..delta.unsigned_abs() {
            if delta > 0 {
                if row + 1 < rows(line) {
                    row += 1;
                } else if line + 1 < self.rope.len_lines() - 1 {
                    line += 1;
                    row = 0;
                } else {
                    break;
                }
            } else if row > 0 {
                row -= 1;
            } else if line > 0 {
                line -= 1;
                row = rows(line) - 1;
            } else {
                break;
            }
        }
        let in_row = cell % width;
        let col = (row * width + in_row).min(self.rope.line_len(line));
        Some(self.rope.line_start(line) + col)
    }
}

/// 把按键交给 Vim 层；插入模式下未被处理的按键按键入的文字执行并记录。
pub fn press(vim: &mut Vim, shared: &mut Shared, host: &mut TestHost, key: Key) {
    if vim.key(shared, host, key) || vim.mode() != Mode::Insert {
        return;
    }
    let input = match key.code() {
        Code::Char(c) if key.as_char().is_some() => Input::Text(c.to_string()),
        Code::Named(Named::Enter) => Input::Newline,
        Code::Named(Named::Tab) => Input::Text("\t".into()),
        Code::Named(Named::Backspace) => Input::Backspace { word: false },
        Code::Named(Named::Delete) => Input::Delete { word: false },
        _ => return,
    };
    host.input(&input);
    vim.typed(input);
}

/// 依次输入按键记法 `keys` 中的按键。
pub fn feed(vim: &mut Vim, shared: &mut Shared, host: &mut TestHost, keys: &str) {
    for key in notation::parse(keys) {
        press(vim, shared, host, key);
    }
}
