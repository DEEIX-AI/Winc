//! Vim 模式的接入：按键的归属与转换、宿主接口、与 Vim 层之外的选区变化的同步。
//!
//! Vim 模式关闭时 [`App::vim`] 为 `None`，按键路径上只多这一次判断，不创建任何状态。寄存器、
//! 上一次修改等各窗口共用的状态保存在进程内的 [`shared`] 中，首次使用时创建。

use std::ops::Range;
use std::sync::{Mutex, MutexGuard, PoisonError};

use winc_config::VimClipboard;
use winc_core::{Change, Indent, Lines, Selection};
use winc_editor::{Command, Content, Document, DocumentId};
use winc_i18n::t;
use winc_vim::{Clipboard, Host, Indentation, Input, Message, Mode, Shared, Vim, VisualKind};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::app::{App, Focus};

/// 返回各窗口共用的 Vim 状态，首次调用时创建。
fn shared() -> MutexGuard<'static, Shared> {
    #[cfg(not(test))]
    let shared = {
        static SHARED: std::sync::OnceLock<Mutex<Shared>> = std::sync::OnceLock::new();
        SHARED.get_or_init(Mutex::default)
    };
    // 测试并行运行，各线程使用各自的状态。
    #[cfg(test)]
    let shared = {
        thread_local!(static SHARED: &'static Mutex<Shared> = Box::leak(Box::default()));
        SHARED.with(|shared| *shared)
    };
    // 持有锁期间 panic 不会使状态不一致到无法继续使用：按键之间没有跨调用的中间状态。
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 一个窗口的 Vim 状态；Vim 模式关闭时不存在。
#[derive(Debug, Default)]
pub(crate) struct VimState {
    /// 模式、未完成的按键等。
    vim: Vim,
    /// 状态所属的文档；切换文档时回到普通模式。
    doc: Option<DocumentId>,
    /// Vim 层最近一次处理或核对之后该文档的选区与版本号；与当前的不同时说明选区或文本被
    /// Vim 层之外的操作改动过。
    seen: Option<(Selection, u64)>,
}

/// 光标的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaretShape {
    /// 覆盖所在字符的实心块：普通与可视模式。
    Block,
    /// 所在字符下方的横线：等待动作。
    Underline,
    /// 空心块：编辑区没有键盘焦点。
    Hollow,
}

/// 宿主所需的视图参数，在借用文档之前算好。
struct View {
    /// 制表符的宽度（显示列）。
    tab_width: usize,
    /// 翻页时移动的行数。
    page: usize,
    /// 窗口中完整显示的文本行。
    visible: Range<usize>,
    /// 文字区一行可以容纳的显示列数。
    text_width: usize,
}

/// 系统剪贴板的访问；测试中以 `App::copied` 代替。
struct Clip<'a> {
    #[cfg(not(test))]
    system: &'a mut Option<arboard::Clipboard>,
    #[cfg(test)]
    copied: &'a mut Option<String>,
    /// 最近一次整行复制的文本：应用的粘贴据此把整行插到当前行的上方。
    whole_line: &'a mut Option<String>,
}

impl Clip<'_> {
    fn get(&mut self) -> Option<String> {
        #[cfg(test)]
        {
            self.copied.clone()
        }
        #[cfg(not(test))]
        {
            if self.system.is_none() {
                *self.system = arboard::Clipboard::new().ok();
            }
            self.system.as_mut()?.get_text().ok()
        }
    }

    fn set(&mut self, text: &str) {
        *self.whole_line = text.ends_with('\n').then(|| text.to_owned());
        #[cfg(test)]
        {
            *self.copied = Some(text.to_owned());
        }
        #[cfg(not(test))]
        {
            if self.system.is_none() {
                *self.system = arboard::Clipboard::new().ok();
            }
            if let Some(clipboard) = self.system.as_mut() {
                let _ = clipboard.set_text(text.to_owned());
            }
        }
    }
}

/// 以一个文档为宿主：Vim 层在其上读取文本、应用修改。
struct DocHost<'a> {
    doc: &'a mut Document,
    view: View,
    clip: Clip<'a>,
    /// 本次按键产生的提示，处理结束后显示。
    messages: Vec<Message>,
}

impl Host for DocHost<'_> {
    type Text = Content;

    fn text(&self) -> &Content {
        self.doc.text()
    }

    fn selection(&self) -> Selection {
        self.doc.selection
    }

    fn set_selection(&mut self, selection: Selection) {
        // 其余光标（多光标）收回：命令只在主选区上求值。
        self.doc.collapse_selections();
        self.doc.selection = selection;
        self.doc.goal_cell = None;
    }

    fn apply(&mut self, changes: Vec<Change>, before: usize, after: usize) -> bool {
        self.doc.apply_changes(changes, before, after)
    }

    fn input(&mut self, input: &Input) {
        let command = match input {
            Input::Text(text) => Command::Insert(text.clone()),
            Input::Paste(text) => Command::Paste(text.clone()),
            Input::Newline => Command::Newline,
            Input::Backspace { word } => Command::Backspace { word: *word },
            Input::Delete { word } => Command::Delete { word: *word },
            Input::DeleteToLineStart => Command::DeleteToLineStart,
            Input::Indent => Command::Indent,
            Input::Outdent => Command::Outdent,
        };
        self.doc
            .execute(command, self.view.page, self.view.tab_width);
    }

    fn open_line(&mut self, at: usize) {
        self.doc.open_line_at(at, self.view.tab_width);
    }

    fn undo(&mut self) -> bool {
        let revision = self.doc.revision;
        (self.doc).execute(Command::Undo, self.view.page, self.view.tab_width);
        self.doc.revision != revision
    }

    fn redo(&mut self) -> bool {
        let revision = self.doc.revision;
        (self.doc).execute(Command::Redo, self.view.page, self.view.tab_width);
        self.doc.revision != revision
    }

    fn redo_origin(&self) -> Option<usize> {
        self.doc.redo_origin()
    }

    fn begin_group(&mut self) {
        self.doc.begin_undo_group();
    }

    fn end_group(&mut self) {
        self.doc.end_undo_group();
    }

    fn read_only(&self) -> bool {
        self.doc.readonly
    }

    fn fits(&self, from: usize, to: usize) -> bool {
        self.doc.fits_edit(from, to)
    }

    fn tab_width(&self) -> usize {
        self.view.tab_width.max(1)
    }

    fn indentation(&self) -> Indentation {
        match self.doc.indent() {
            Indent::Tabs => Indentation {
                width: self.view.tab_width.max(1),
                tabs: true,
            },
            Indent::Spaces(width) => Indentation {
                width: width.max(1),
                tabs: false,
            },
        }
    }

    fn keeps_indent(&self) -> bool {
        true
    }

    fn line_ending(&self) -> &str {
        self.doc.line_ending.as_str()
    }

    fn visible_lines(&self) -> Range<usize> {
        self.view.visible.clone()
    }

    fn text_width(&self) -> usize {
        self.view.text_width
    }

    fn display_row(&self, pos: usize) -> Range<usize> {
        self.doc.display_row_span(pos)
    }

    fn display_vertical(&self, pos: usize, delta: isize, _cell: usize) -> Option<usize> {
        self.doc.display_vertical(pos, delta)
    }

    fn has_folds(&self) -> bool {
        self.doc.has_collapsed_folds()
    }

    fn fold(&self, line: usize) -> Option<(usize, usize)> {
        self.doc.collapsed_fold_at(line)
    }

    fn clipboard(&mut self) -> Option<String> {
        self.clip.get()
    }

    fn set_clipboard(&mut self, text: &str) {
        self.clip.set(text);
    }

    fn toggle_comment(&mut self, first: usize, last: usize) -> bool {
        (self.doc).toggle_comment_lines(first, last, self.view.tab_width)
    }

    fn file_name(&self) -> Option<String> {
        self.doc.path.as_ref().map(|_| self.doc.name().into_owned())
    }

    fn message(&mut self, message: Message) {
        self.messages.push(message);
    }
}

/// Windows、Linux 上默认留给应用的 `Ctrl` 组合：复制、粘贴、剪切、全选、保存、撤销、查找、
/// 新建、快速打开。
const APP_KEYS: &str = "cvxaszfnp";

/// 按键 `key` 在模式 `mode` 下是否交给 Vim 层。
///
/// 带 `Command` 的组合属于应用。`Ctrl` 与字符的组合先查 `overrides`（按键名与是否交给 Vim 层），
/// 其次按默认规则：macOS 上全部交给 Vim 层；其他系统上 [`APP_KEYS`] 属于应用，`Ctrl+W` 只在
/// 插入模式交给 Vim 层。交给 Vim 层的按键在该模式下没有定义时仍然回到应用。
pub(crate) fn claims(
    key: winc_vim::Key,
    mode: Mode,
    mac: bool,
    overrides: &[(String, bool)],
) -> bool {
    use winc_vim::{Code, Mods};
    let mods = key.mods();
    if mods.contains(Mods::CMD) {
        return false;
    }
    let Code::Char(c) = key.code() else {
        return true;
    };
    if !mods.contains(Mods::CTRL) {
        return true;
    }
    let name = format!("ctrl+{c}");
    if let Some((_, vim)) = overrides.iter().find(|(key, _)| *key == name) {
        return *vim;
    }
    if mac {
        return true;
    }
    match c {
        'w' => mode == Mode::Insert,
        c => !APP_KEYS.contains(c),
    }
}

/// 返回美式布局上按住 `Shift` 时字符 `c` 所在的键产生的字符。
fn us_shifted(c: char) -> char {
    const PLAIN: &str = "`1234567890-=[]\\;',./";
    const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";
    match PLAIN.find(c) {
        Some(index) => SHIFTED[index..].chars().next().unwrap_or(c),
        None => c.to_ascii_uppercase(),
    }
}

/// 把窗口的按键转换成 Vim 层的按键；不属于 Vim 层处理的范围（单独的修饰键、功能键 `F1` 等）
/// 时返回 `None`。
///
/// `logical` 是按键产生的键，`plain` 是不计修饰键时产生的键，`physical` 是物理键位。`Ctrl`
/// 组合按不含修饰键的字符识别；`latin` 为真时（普通与可视模式），产生非 ASCII 字符的键按物理
/// 键位取美式布局的字符，使非拉丁布局下命令仍然可用。
pub(crate) fn convert(
    logical: &Key,
    plain: &Key,
    physical: PhysicalKey,
    modifiers: ModifiersState,
    latin: bool,
) -> Option<winc_vim::Key> {
    use winc_vim::{Code, Mods, Named};
    let mut mods = Mods::NONE;
    for (pressed, flag) in [
        (modifiers.control_key(), Mods::CTRL),
        (modifiers.shift_key(), Mods::SHIFT),
        (modifiers.alt_key(), Mods::ALT),
        (modifiers.super_key(), Mods::CMD),
    ] {
        if pressed {
            mods = mods.with(flag);
        }
    }
    let named = |named: Named| Some(winc_vim::Key::new(Code::Named(named), mods));
    match logical {
        Key::Named(key) => match key {
            NamedKey::Escape => named(Named::Esc),
            NamedKey::Enter => named(Named::Enter),
            NamedKey::Tab => named(Named::Tab),
            NamedKey::Backspace => named(Named::Backspace),
            NamedKey::Delete => named(Named::Delete),
            NamedKey::Insert => named(Named::Insert),
            NamedKey::ArrowUp => named(Named::Up),
            NamedKey::ArrowDown => named(Named::Down),
            NamedKey::ArrowLeft => named(Named::Left),
            NamedKey::ArrowRight => named(Named::Right),
            NamedKey::Home => named(Named::Home),
            NamedKey::End => named(Named::End),
            NamedKey::PageUp => named(Named::PageUp),
            NamedKey::PageDown => named(Named::PageDown),
            NamedKey::Space => Some(winc_vim::Key::new(Code::Char(' '), mods)),
            _ => None,
        },
        Key::Character(text) => {
            let typed = text.chars().next()?;
            let ctrl = modifiers.control_key() && !modifiers.alt_key();
            if ctrl || modifiers.super_key() {
                // 带 `Ctrl` 或 `Command` 时取不含修饰键的字符，非拉丁布局下取物理键位的字符。
                let base = match plain {
                    Key::Character(text) => text.chars().next().filter(char::is_ascii),
                    _ => None,
                };
                let c = base.or_else(|| crate::keymap::us_char(physical))?;
                return Some(winc_vim::Key::new(Code::Char(c), mods.without(Mods::ALT)));
            }
            // 其余的按键以产生的字符识别，`Shift` 与 `Alt` 已体现在字符中；`Alt` 产生的非 ASCII
            // 字符不是布局所致，不按物理键位取字符。
            let c = if latin && !typed.is_ascii() && !modifiers.alt_key() {
                match crate::keymap::us_char(physical) {
                    Some(c) if modifiers.shift_key() => us_shifted(c),
                    Some(c) => c,
                    None => typed,
                }
            } else {
                typed
            };
            Some(winc_vim::Key::char(c))
        }
        _ => None,
    }
}

impl App {
    /// 使 Vim 状态的有无与配置一致：开启时创建，关闭时结束未完成的撤销分组并销毁。
    pub(in crate::app) fn vim_apply_config(&mut self) {
        let enabled = self.config.vim_enabled == Some(true);
        if enabled && self.vim.is_none() {
            self.vim = Some(Box::default());
        } else if !enabled && self.vim.is_some() {
            if let Some(mut state) = self.vim.take() {
                self.vim_leave(&mut state);
            }
            self.request_redraw();
        }
        if enabled {
            shared().clipboard = match self.config.vim_clipboard.unwrap_or_default() {
                VimClipboard::Register => Clipboard::Register,
                VimClipboard::Yank => Clipboard::Yank,
                VimClipboard::Always => Clipboard::Always,
            };
        }
    }

    /// 当前焦点与文档是否适用 Vim 模式：焦点在编辑区，文档以文本显示。
    fn vim_applies(&self) -> bool {
        self.focus == Focus::Editor && self.editor.active().is_some_and(Document::shows_text)
    }

    /// 返回 Vim 层的模式；Vim 模式关闭或不适用于当前文档时返回 `None`。
    pub(crate) fn vim_mode(&self) -> Option<Mode> {
        let state = self.vim.as_ref()?;
        self.vim_applies().then(|| state.vim.mode())
    }

    /// 返回 Vim 模式下光标的位置与形状；使用普通的竖线光标时（关闭、不适用、插入模式）返回
    /// `None`。
    pub(crate) fn vim_caret(&self) -> Option<(usize, CaretShape)> {
        let state = self.vim.as_ref()?;
        let doc = self.editor.active().filter(|doc| doc.shows_text())?;
        if state.doc != Some(doc.id) || state.vim.mode() == Mode::Insert {
            return None;
        }
        let end = doc.text().end_pos();
        let pos = match state.vim.visual_kind() {
            // 可视模式的光标在选区的光标一端所含的字符上。
            Some(_) if doc.selection.head > doc.selection.anchor => {
                winc_core::movement::prev_grapheme(doc.text(), doc.selection.head.min(end))
            }
            _ => doc.selection.head.min(end),
        };
        let shape = if self.focus != Focus::Editor || !self.window.focused {
            CaretShape::Hollow
        } else if state.vim.is_operator_pending() {
            CaretShape::Underline
        } else {
            CaretShape::Block
        };
        Some((pos, shape))
    }

    /// 返回状态栏上显示的 Vim 状态：模式（普通模式不显示）与未完成的按键；没有内容时返回
    /// `None`。
    pub(crate) fn vim_status(&self) -> Option<String> {
        let state = self.vim.as_ref()?;
        if !self.vim_applies() {
            return None;
        }
        let mode = match (state.vim.mode(), state.vim.visual_kind()) {
            (Mode::Insert, _) => t!("workspace-status-vim-insert"),
            (_, Some(VisualKind::Char)) => t!("workspace-status-vim-visual"),
            (_, Some(VisualKind::Line)) => t!("workspace-status-vim-visual-line"),
            _ => "",
        };
        let pending = winc_vim::notation::write(state.vim.pending());
        let text = match (mode.is_empty(), pending.is_empty()) {
            (true, true) => return None,
            (false, true) => format!("-- {mode} --"),
            (true, false) => pending,
            (false, false) => format!("-- {mode} --  {pending}"),
        };
        Some(text)
    }

    /// 返回宿主所需的视图参数。
    fn vim_view(&self) -> View {
        let tab_width = self.editor.active_tab_width();
        let rows = self.visible_lines();
        let visible = match self.editor.active() {
            Some(doc) => {
                let first = doc.scroll_line.max(0.0).ceil() as usize;
                let top = doc.line_of_row(first);
                let bottom = doc.line_of_row(first + rows.saturating_sub(1));
                top..bottom + 1
            }
            None => 0..1,
        };
        View {
            tab_width,
            page: rows.saturating_sub(1).max(1),
            visible,
            text_width: ((self.layout.text.w / self.cell.max(1.0)) as usize).max(1),
        }
    }

    /// 在文档 `doc` 上回到普通模式并结束未完成的撤销分组；文档已不存在时只丢弃状态。
    fn vim_leave(&mut self, state: &mut VimState) {
        let view = self.vim_view();
        let clip = Clip {
            #[cfg(not(test))]
            system: &mut self.clipboard,
            #[cfg(test)]
            copied: &mut self.copied,
            whole_line: &mut self.line_clipboard,
        };
        let documents = &mut self.editor.documents;
        match documents.iter_mut().find(|doc| Some(doc.id) == state.doc) {
            Some(doc) => {
                let mut host = DocHost {
                    doc,
                    view,
                    clip,
                    messages: Vec::new(),
                };
                state.vim.reset(&mut host);
            }
            None => state.vim.detach(),
        }
        state.doc = None;
        state.seen = None;
    }

    /// 在当前文档上以 `f` 操作 Vim 层，返回 `f` 的结果与期间产生的提示；没有文档时返回 `None`。
    fn vim_with<R>(
        &mut self,
        state: &mut VimState,
        f: impl FnOnce(&mut Vim, &mut Shared, &mut DocHost<'_>) -> R,
    ) -> Option<(R, Vec<Message>)> {
        let view = self.vim_view();
        let clip = Clip {
            #[cfg(not(test))]
            system: &mut self.clipboard,
            #[cfg(test)]
            copied: &mut self.copied,
            whole_line: &mut self.line_clipboard,
        };
        let doc = self.editor.active_mut()?;
        let mut host = DocHost {
            doc,
            view,
            clip,
            messages: Vec::new(),
        };
        let result = f(&mut state.vim, &mut shared(), &mut host);
        state.seen = Some((host.doc.selection, host.doc.revision));
        Some((result, host.messages))
    }

    /// 使 Vim 层的状态与当前文档一致：切换文档时回到普通模式；选区或文本被 Vim 层之外的操作
    /// （鼠标、应用的命令）改动之后，普通模式下按选区进入或离开可视模式，插入模式下另起一个
    /// 撤销单元。每次绘制之前与每次按键之前调用。
    pub(in crate::app) fn vim_sync(&mut self) {
        let Some(mut state) = self.vim.take() else {
            return;
        };
        let active = self.editor.active().map(|doc| doc.id);
        if state.doc != active {
            self.vim_leave(&mut state);
            state.doc = active;
        }
        if self.vim_applies() {
            let now = self
                .editor
                .active()
                .map(|doc| (doc.selection, doc.revision));
            if state.seen != now {
                self.vim_with(&mut state, |vim, _, host| {
                    if vim.mode() == Mode::Insert {
                        vim.insert_moved(host);
                    } else {
                        vim.sync(host);
                    }
                });
            }
        } else {
            // 焦点离开编辑区：未完成的按键取消，模式不变。
            state.vim.cancel_pending();
        }
        self.vim = Some(state);
    }

    /// 处理窗口的按键 `event`，返回是否已由 Vim 层处理。
    pub(in crate::app) fn vim_key_event(&mut self, event: &KeyEvent) -> bool {
        self.vim_sync();
        let Some(mode) = self.vim_mode() else {
            return false;
        };
        // 编辑区上方的改动卡片先由 `Esc` 关闭。
        let escape = event.logical_key == Key::Named(NamedKey::Escape);
        if escape && self.changes.popup.is_some() {
            return false;
        }
        let key = convert(
            &event.logical_key,
            &event.key_without_modifiers(),
            event.physical_key,
            self.modifiers,
            mode != Mode::Insert,
        );
        let Some(key) = key else {
            return false;
        };
        let mac = cfg!(target_os = "macos");
        if !claims(key, mode, mac, &self.config.vim_handle_keys) {
            return false;
        }
        self.vim_press(key)
    }

    /// 把按键 `key` 交给 Vim 层，返回是否已处理；不适用于当前文档时返回假。
    pub(in crate::app) fn vim_press(&mut self, key: winc_vim::Key) -> bool {
        self.vim_sync();
        if !self.vim_applies() {
            return false;
        }
        let Some(mut state) = self.vim.take() else {
            return false;
        };
        let lines = self
            .editor
            .active()
            .map_or(0, |doc| doc.text().line_count());
        let result = self.vim_with(&mut state, |vim, shared, host| vim.key(shared, host, key));
        self.vim = Some(state);
        let Some((claimed, messages)) = result else {
            return false;
        };
        if claimed {
            self.vim_after(lines, messages);
        }
        claimed
    }

    /// 依次把按键记法 `keys` 中的按键交给 Vim 层；应用的复制、剪切、粘贴、撤销、重做经此执行。
    pub(in crate::app) fn vim_feed(&mut self, keys: &str) {
        for key in winc_vim::notation::parse(keys) {
            self.vim_press(key);
        }
    }

    /// Vim 层处理按键之后更新界面：显示提示，行数变化时重新布局，滚动到光标。
    fn vim_after(&mut self, lines_before: usize, messages: Vec<Message>) {
        if let Some(message) = messages.last() {
            let text = match message {
                Message::Unsupported => t!("editor-vim-unsupported").to_owned(),
                Message::TooLarge => t!("editor-vim-too-large").to_owned(),
                Message::ReadOnly => {
                    let reason = self.editor.active().and_then(Document::readonly_reason);
                    reason.unwrap_or(t!("workspace-status-readonly")).to_owned()
                }
                Message::EmptyRegister(name) => {
                    t!("editor-vim-register-empty", name = name.to_string())
                }
                Message::ReadOnlyRegister(name) => {
                    t!("editor-vim-register-read-only", name = name.to_string())
                }
            };
            self.flash(text);
        }
        if self
            .editor
            .active()
            .map_or(0, |doc| doc.text().line_count())
            != lines_before
        {
            self.relayout();
        }
        self.ensure_caret_visible();
        self.reset_blink();
        self.request_redraw();
    }

    /// 编辑命令 `command` 是插入模式中的一次输入时返回对应的输入，供执行之后记录。
    pub(in crate::app) fn vim_input(&self, command: &Command) -> Option<Input> {
        let state = self.vim.as_ref()?;
        if state.vim.mode() != Mode::Insert || !self.vim_applies() {
            return None;
        }
        Some(match command {
            Command::Insert(text) => Input::Text(text.clone()),
            Command::Paste(text) | Command::InsertLinesAbove(text) => Input::Paste(text.clone()),
            Command::Newline => Input::Newline,
            Command::Backspace { word } => Input::Backspace { word: *word },
            Command::Delete { word } => Input::Delete { word: *word },
            Command::DeleteToLineStart => Input::DeleteToLineStart,
            Command::Indent => Input::Indent,
            Command::Outdent => Input::Outdent,
            _ => return None,
        })
    }

    /// 记录插入模式中已经执行的输入 `input`，供次数与点重复重放。
    pub(in crate::app) fn vim_typed(&mut self, input: Input) {
        let now = self
            .editor
            .active()
            .map(|doc| (doc.selection, doc.revision));
        if let Some(state) = &mut self.vim {
            state.vim.typed(input);
            state.seen = now;
        }
    }

    /// Vim 模式下应用的复制、剪切、粘贴、撤销、重做改由 Vim 层执行时返回对应的按键；插入模式
    /// 与不适用时返回 `None`，由应用按原样执行。
    pub(in crate::app) fn vim_equivalent(
        &mut self,
        action: crate::keymap::Action,
    ) -> Option<&'static str> {
        use crate::keymap::Action;
        self.vim_sync();
        let visual = match self.vim_mode()? {
            Mode::Insert => return None,
            mode => mode == Mode::Visual,
        };
        Some(match action {
            Action::Copy if visual => "\"+y",
            Action::Copy => "\"+yy",
            Action::Cut if visual => "\"+d",
            Action::Cut => "\"+dd",
            Action::Paste => "\"+gP",
            Action::Undo => "u",
            Action::Redo => "<C-r>",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;
