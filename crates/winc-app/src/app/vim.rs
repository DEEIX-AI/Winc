//! Vim 模式的接入：按键的归属与转换、宿主接口、与 Vim 层之外的选区变化的同步。
//!
//! Vim 模式关闭时 [`App::vim`] 为 `None`，按键路径上只多这一次判断，不创建任何状态。寄存器、
//! 上一次修改等各窗口共用的状态保存在进程内的 [`shared`] 中，首次使用时创建。

mod command;
mod host;

use std::sync::{Mutex, MutexGuard, PoisonError};

use winc_config::VimClipboard;
use winc_core::{Lines, Selection};
use winc_editor::{Command, Document, DocumentId, ModalPattern};
use winc_i18n::t;
use winc_vim::{Clipboard, Input, Message, Mode, Place, Prompt, Query, Shared, Vim, VisualKind};

pub(crate) use command::VimList;
use host::{Clip, Deferred, DocHost, View, visible_lines};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::app::{App, Focus, PromptKind};
use winc_render::Rect;

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
    /// 最近一次查找的模式及其编译结果。
    pattern: Option<(Query, ModalPattern)>,
    /// 是否高亮最近一次查找的匹配。
    highlight: bool,
    /// 查找输入框中最近一次用于即时跳转的文本。
    previewed: Option<String>,
    /// 命令行输入条中进行中的历史翻阅与补全。
    line: command::Line,
    /// 映射等待后续按键超时的时刻。
    map_deadline: Option<std::time::Instant>,
}

/// 光标的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaretShape {
    /// 覆盖所在字符的实心块：普通与可视模式。
    Block,
    /// 所在字符下方的横线：等待动作与替换模式。
    Underline,
    /// 空心块：编辑区没有键盘焦点。
    Hollow,
}

/// 一次按键在位置历史中最多移动的步数（步）；次数更大时按该值执行。
///
/// 取自调参：不小于位置历史保留的条数。
const MAX_JUMP_STEPS: usize = 64;

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
            let mut shared = shared();
            shared.clipboard = match self.config.vim_clipboard.unwrap_or_default() {
                VimClipboard::Register => Clipboard::Register,
                VimClipboard::Yank => Clipboard::Yank,
                VimClipboard::Always => Clipboard::Always,
            };
            shared.ignore_case = self.config.vim_ignore_case == Some(true);
            shared.smart_case = self.config.vim_smart_case == Some(true);
            shared.set_mappings(self.vim_mappings());
        }
    }

    /// 是否高亮 Vim 层最近一次查找的匹配：查找条没有打开，且查找内容仍是该模式。
    pub(crate) fn vim_highlights(&self) -> bool {
        let Some(state) = &self.vim else {
            return false;
        };
        state.highlight
            && shared().options.hlsearch
            && !self.find.open
            && self.search.regex
            && (state.pattern.as_ref()).is_some_and(|(query, _)| query.regex == self.search.query)
    }

    /// 返回输入条的位置：Vim 模式的命令行与查找占状态栏所在的一行，其余位于编辑区顶部居中。
    pub(crate) fn prompt_bar(&self) -> Rect {
        if self.prompt_kind.is_vim() {
            self.layout.status
        } else {
            crate::layout::goto_bar(self.layout.editor)
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
        let replace = state.vim.is_replace();
        if state.doc != Some(doc.id) || (state.vim.mode() == Mode::Insert && !replace) {
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
        } else if state.vim.is_operator_pending() || replace {
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
            (Mode::Insert, _) if state.vim.is_replace() => t!("workspace-status-vim-replace"),
            (Mode::Insert, _) => t!("workspace-status-vim-insert"),
            (_, Some(VisualKind::Char)) => t!("workspace-status-vim-visual"),
            (_, Some(VisualKind::Line)) => t!("workspace-status-vim-visual-line"),
            (_, Some(VisualKind::Block)) => t!("workspace-status-vim-visual-block"),
            _ => "",
        };
        // 插入模式中执行一条普通模式的命令期间，模式写在括号里。
        let mode = if state.vim.is_suspended() && mode.is_empty() {
            format!("({})", t!("workspace-status-vim-insert"))
        } else {
            mode.to_owned()
        };
        let mut pending = winc_vim::notation::write(state.vim.pending());
        // 逐处确认时显示替换之后的文本与可用的回答；记录宏时显示寄存器名。
        if let Some(replacement) = state.vim.confirming() {
            pending = t!("workspace-status-vim-confirm", text = replacement);
        } else if let Some(name) = state.vim.recording() {
            let recording = t!("workspace-status-vim-recording", name = name.to_string());
            pending = format!("{recording}  {pending}").trim_end().to_owned();
        }
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
            Some(doc) => visible_lines(doc, rows),
            None => 0..1,
        };
        View {
            tab_width,
            page: rows.saturating_sub(1).max(1),
            visible,
            text_width: ((self.layout.text.w / self.cell.max(1.0)) as usize).max(1),
            rows,
            max_scroll: self.max_scroll_line(),
            cell: self.cell,
            scroll_off: self.vim_scroll_off(),
            numbers: self.config.line_numbers != Some(false),
            whitespace: self.config.render_whitespace == Some(winc_config::RenderWhitespace::All),
        }
    }

    /// 返回光标与窗口上下边缘之间保留的行数：`:set` 设置的值优先于配置。
    fn vim_scroll_off(&self) -> usize {
        let configured = self.config.vim_scroll_off.unwrap_or(0);
        shared().options.scrolloff.unwrap_or(configured)
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
                    pattern: &mut state.pattern,
                    deferred: Vec::new(),
                };
                state.vim.reset(&mut host);
            }
            None => state.vim.detach(),
        }
        state.doc = None;
        state.seen = None;
        state.highlight = false;
        state.previewed = None;
        state.map_deadline = None;
    }

    /// 在当前文档上以 `f` 操作 Vim 层，返回 `f` 的结果、期间产生的提示与留待应用执行的动作；
    /// 没有文档时返回 `None`。
    fn vim_with<R>(
        &mut self,
        state: &mut VimState,
        f: impl FnOnce(&mut Vim, &mut Shared, &mut DocHost<'_>) -> R,
    ) -> Option<(R, Vec<Message>, Vec<Deferred>)> {
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
            pattern: &mut state.pattern,
            deferred: Vec::new(),
        };
        let result = f(&mut state.vim, &mut shared(), &mut host);
        state.seen = Some((host.doc.selection, host.doc.revision));
        Some((result, host.messages, host.deferred))
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
        // 查找模式的输入条：输入的文本变化时即时跳转；输入条被其他操作关闭时放弃输入。
        let prompting = self.vim_prompt_open() && self.focus == Focus::Prompt;
        if state.vim.prompt().is_some() {
            if prompting {
                let text = (self.goto_input.as_ref()).map(|field| field.text().to_owned());
                let searching = self.prompt_kind != PromptKind::VimCommand;
                if searching && state.previewed != text {
                    let typed = text.clone().unwrap_or_default();
                    let result = self.vim_with(&mut state, |vim, shared, host| {
                        vim.prompt_preview(shared, host, &typed);
                    });
                    for action in result.map(|(_, _, deferred)| deferred).unwrap_or_default() {
                        if let Deferred::Highlight(query) = action {
                            self.vim_set_highlight(&query);
                            state.highlight = true;
                        }
                    }
                    state.previewed = text;
                    self.ensure_caret_visible();
                }
            } else {
                // 输入条被其他操作关闭：放弃输入，光标留在当前的位置。
                state.vim.prompt_abandon();
                state.previewed = None;
            }
        }
        if state.vim.prompt().is_some() {
            self.vim = Some(state);
            return;
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
        let handled = self.vim_press(key);
        // 普通模式下没有可取消的内容时，`Esc` 同时取消查找匹配的高亮。
        if !handled
            && escape
            && let Some(state) = &mut self.vim
            && state.highlight
        {
            state.highlight = false;
            self.request_redraw();
        }
        handled
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
        let prompt = state.vim.prompt();
        self.vim = Some(state);
        let Some((claimed, messages, deferred)) = result else {
            return false;
        };
        if claimed {
            self.vim_after(lines, messages, deferred);
        }
        if let Some(prompt) = prompt {
            self.vim_open_prompt(prompt);
        }
        self.vim_await_mapping();
        claimed
    }

    /// 输入条按下回车：以 `text` 为命令行或查找的模式，执行等待中的命令。
    pub(in crate::app) fn vim_prompt_submit(&mut self, text: &str) {
        let Some(mut state) = self.vim.take() else {
            return;
        };
        let lines = (self.editor.active()).map_or(0, |doc| doc.text().line_count());
        let result = self.vim_with(&mut state, |vim, shared, host| {
            vim.prompt_submit(shared, host, text);
        });
        state.previewed = None;
        // 命令行中执行的宏可以再次等待输入。
        let prompt = state.vim.prompt();
        self.vim = Some(state);
        if let Some(((), messages, deferred)) = result {
            self.vim_after(lines, messages, deferred);
        }
        if let Some(prompt) = prompt {
            self.vim_open_prompt(prompt);
        }
        self.vim_await_mapping();
    }

    /// 放弃输入：查找时光标回到输入之前的位置。
    pub(in crate::app) fn vim_prompt_cancel(&mut self) {
        let Some(mut state) = self.vim.take() else {
            return;
        };
        let searching = matches!(state.vim.prompt(), Some(Prompt::Search { .. }));
        self.vim_with(&mut state, |vim, _, host| vim.prompt_cancel(host));
        state.previewed = None;
        state.highlight &= !searching;
        self.vim = Some(state);
        self.ensure_caret_visible();
        self.request_redraw();
    }

    /// 替换模式下键入文字的命令 `command` 由 Vim 层执行时返回真：文字覆盖光标处的字符。
    pub(in crate::app) fn vim_overtype(&mut self, command: &Command) -> bool {
        let Command::Insert(text) = command else {
            return false;
        };
        let replacing = (self.vim.as_ref()).is_some_and(|state| state.vim.is_replace());
        if !replacing || !self.vim_applies() {
            return false;
        }
        let Some(mut state) = self.vim.take() else {
            return false;
        };
        let lines = (self.editor.active()).map_or(0, |doc| doc.text().line_count());
        let result = self.vim_with(&mut state, |vim, _, host| vim.overtype(host, text));
        self.vim = Some(state);
        if let Some(((), messages, deferred)) = result {
            self.vim_after(lines, messages, deferred);
        }
        true
    }

    /// 依次把按键记法 `keys` 中的按键交给 Vim 层；应用的复制、剪切、粘贴、撤销、重做经此执行。
    pub(in crate::app) fn vim_feed(&mut self, keys: &str) {
        for key in winc_vim::notation::parse(keys) {
            self.vim_press(key);
        }
    }

    /// Vim 层处理按键之后更新界面：执行留待应用执行的动作，显示提示，行数变化时重新布局，
    /// 滚动到光标。
    fn vim_after(&mut self, lines_before: usize, messages: Vec<Message>, deferred: Vec<Deferred>) {
        let mut revealed = false;
        for action in deferred {
            match action {
                Deferred::Format(first, last) => self.vim_format(first, last),
                Deferred::Jump(back, count) => {
                    for _ in 0..count.min(MAX_JUMP_STEPS) {
                        self.navigate(!back);
                    }
                }
                Deferred::MarkJump => self.navigation.mark_jump(),
                Deferred::Reveal(line, place) => {
                    self.vim_reveal(line, place);
                    revealed = true;
                }
                Deferred::Fold(command) => self.run_fold(command),
                Deferred::Highlight(query) => {
                    self.vim_set_highlight(&query);
                    if let Some(state) = &mut self.vim {
                        state.highlight = true;
                    }
                }
                Deferred::Set(option, value) => self.vim_set_option(option, value),
                Deferred::App(command) => self.vim_run_app(command),
            }
        }
        if let Some(message) = messages.last() {
            let text = self.vim_message(message);
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
        // 滚动命令已把所在行放到指定的位置，不再为光标调整滚动位置。
        if !revealed {
            self.ensure_caret_visible();
            self.vim_keep_margin();
        }
        self.reset_blink();
        self.request_redraw();
    }

    /// 返回提示 `message` 的文案。
    fn vim_message(&self, message: &Message) -> String {
        let count = |count: &usize| count.to_string();
        match message {
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
            Message::NotFound => t!("editor-vim-not-found").to_owned(),
            Message::BadPattern(at) => {
                t!("editor-vim-bad-pattern", column = (at + 1).to_string())
            }
            Message::NoSearch => t!("editor-vim-no-search").to_owned(),
            Message::NoMark(name) => t!("editor-vim-no-mark", name = name.to_string()),
            Message::BadCommand => t!("editor-vim-bad-command").to_owned(),
            Message::BadArgument => t!("editor-vim-bad-argument").to_owned(),
            Message::BadRange => t!("editor-vim-bad-range").to_owned(),
            Message::NoSubstitute => t!("editor-vim-no-substitute").to_owned(),
            Message::NoCommand => t!("editor-vim-no-command").to_owned(),
            Message::Interrupted => t!("editor-vim-interrupted").to_owned(),
            Message::UnknownOption(name) => t!("editor-vim-unknown-option", name = name.as_str()),
            Message::Substituted { count: done, lines } if *lines > 0 => t!(
                "editor-vim-substituted-lines",
                count = count(done),
                lines = count(lines)
            ),
            Message::Substituted { count: done, .. } => {
                t!("editor-vim-substituted", count = count(done))
            }
            Message::Matched {
                count: found,
                lines,
            } => t!(
                "editor-vim-matched",
                count = count(found),
                lines = count(lines)
            ),
            Message::LineDelta(delta) if *delta > 0 => {
                t!("editor-vim-more-lines", count = delta.to_string())
            }
            Message::LineDelta(delta) => {
                t!(
                    "editor-vim-fewer-lines",
                    count = delta.unsigned_abs().to_string()
                )
            }
            Message::Option(shown) => shown.clone(),
            Message::Char(c) => {
                let code = u32::from(*c);
                format!("<{c}> {code}, Hex {code:x}, Oct {code:o}")
            }
        }
    }

    /// 把 `query` 设为查找条的查找内容：查找条与 Vim 层的查找是同一份查找状态。
    fn vim_set_highlight(&mut self, query: &Query) {
        self.search.query.clone_from(&query.regex);
        self.search.regex = true;
        self.search.case_sensitive = !query.ignore_case;
        self.search.whole_word = false;
        self.find.input.set_text(&query.regex);
    }

    /// 格式化第 `first` 至 `last` 行：复用格式化选区，之后选区收回为光标。
    fn vim_format(&mut self, first: usize, last: usize) {
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let original = doc.selection;
        let text = doc.text();
        let last = last.min(text.line_count().saturating_sub(1));
        let (from, to) = (text.line_start(first.min(last)), text.line_end(last));
        doc.selection = Selection::new(from, to);
        self.format_selection();
        if let Some(doc) = self.editor.active_mut() {
            doc.selection = original;
        }
    }

    /// 纵向滚动到第 `line` 行位于窗口的 `place`；窗口顶部被固定显示的行占用的部分不算。
    fn vim_reveal(&mut self, line: usize, place: Place) {
        let visible = self.visible_lines() as f64;
        let top = (self.sticky_depth(line) as f64).max(1.0);
        let max = self.max_scroll_line();
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let row = doc.row_of(line.min(doc.text().line_count().saturating_sub(1))) as f64;
        let before = doc.scroll_line;
        let target = match place {
            Place::Top => row - top,
            Place::Center => row - (visible / 2.0).floor(),
            Place::Bottom => row - visible + 2.0,
        };
        doc.scroll_line = target.clamp(0.0, max.max(0.0));
        self.glide_scroll(before);
    }

    /// 按配置在光标与窗口上下边缘之间保留若干行。
    fn vim_keep_margin(&mut self) {
        let margin = self.vim_scroll_off();
        if margin == 0 {
            return;
        }
        let visible = self.visible_lines();
        let max = self.max_scroll_line();
        let margin = margin.min(visible.saturating_sub(1) / 2) as f64;
        let Some(doc) = self.editor.active_mut() else {
            return;
        };
        let row = doc.row_at(doc.selection.head) as f64;
        let before = doc.scroll_line;
        let low = row - (visible as f64 - 1.0 - margin);
        let high = row - margin;
        doc.scroll_line = before.clamp(low.min(high), high).clamp(0.0, max.max(0.0));
        if doc.scroll_line != before {
            self.glide_scroll(before);
        }
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
            Command::IndentLines => Input::IndentLine,
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
