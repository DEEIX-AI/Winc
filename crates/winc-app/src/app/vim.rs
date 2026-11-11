//! Vim 模式的接入：按键的归属与转换、宿主接口、与 Vim 层之外的选区变化的同步。
//!
//! Vim 模式关闭时 [`App::vim`] 为 `None`，按键路径上只多这一次判断，不创建任何状态。寄存器、
//! 上一次修改等各窗口共用的状态保存在进程内的 [`shared`] 中，首次使用时创建。

use std::ops::Range;
use std::sync::{Mutex, MutexGuard, PoisonError};

use winc_config::VimClipboard;
use winc_core::{Change, Indent, Lines, Selection};
use winc_editor::{Command, Content, Document, DocumentId, FoldCommand, ModalPattern};
use winc_i18n::t;
use winc_vim::{Clipboard, Fold, Host, Indentation, Input, Message, Mode, Place, Prompt, Query};
use winc_vim::{Scroll, Shared, Vim, VisualKind};
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use crate::app::{App, Focus, PromptKind};
use crate::field::TextField;
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
    /// 文本区可以容纳的显示行数。
    rows: usize,
    /// 纵向滚动位置的上限（显示行）。
    max_scroll: f64,
    /// 一个显示列的宽度（逻辑像素）。
    cell: f32,
    /// 光标与窗口上下边缘之间保留的行数。
    scroll_off: usize,
}

/// Vim 层请求、在按键处理结束之后由应用执行的动作。
#[derive(Debug, Clone, PartialEq)]
enum Deferred {
    /// 格式化第 `first` 至 `last` 行。
    Format(usize, usize),
    /// 在位置历史中后退（真）或前进若干步。
    Jump(bool, usize),
    /// 下一次记录位置时无论远近都算作跳转。
    MarkJump,
    /// 滚动到某一行位于窗口的指定位置。
    Reveal(usize, Place),
    /// 执行折叠命令。
    Fold(FoldCommand),
    /// 把查找的模式设为当前高亮的查找内容。
    Highlight(Query),
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
    /// 最近一次查找的模式及其编译结果。
    pattern: &'a mut Option<(Query, ModalPattern)>,
    /// 处理结束后由应用执行的动作。
    deferred: Vec<Deferred>,
}

/// 返回文档 `doc` 在可以容纳 `rows` 个显示行的窗口中，光标可以停留的文本行。
///
/// 窗口的第一行与最后一行留作边距，与滚动到光标时保留的边距一致；窗口在文本开头时第一行
/// 不留。
fn visible_lines(doc: &Document, rows: usize) -> Range<usize> {
    let first = doc.scroll_line.max(0.0).ceil() as usize;
    let (above, below) = if rows > 3 {
        (usize::from(first > 0), 1)
    } else {
        (0, 0)
    };
    let top = doc.line_of_row(first + above);
    let bottom = doc.line_of_row(first + rows.saturating_sub(1 + below));
    top..bottom.max(top) + 1
}

impl DocHost<'_> {
    /// 按文档当前的滚动位置重新计算窗口中显示的行。
    fn refresh_visible(&mut self) {
        self.view.visible = visible_lines(self.doc, self.view.rows);
    }
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
        if self.doc.has_cursors() {
            self.doc.set_primary(selection);
        } else {
            self.doc.selection = selection;
        }
        self.doc.goal_cell = None;
    }

    fn selections(&self) -> Vec<Selection> {
        if self.doc.has_cursors() {
            self.doc.selections().0
        } else {
            vec![self.doc.selection]
        }
    }

    fn set_selections(&mut self, selections: Vec<Selection>, primary: usize) {
        match selections.as_slice() {
            [only] => {
                self.doc.collapse_selections();
                self.doc.selection = *only;
            }
            _ => self.doc.set_selections(selections, primary),
        }
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
            Input::IndentLine => Command::IndentLines,
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

    fn window_height(&self) -> usize {
        self.view.rows.max(1)
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

    fn find(&mut self, query: &Query, from: usize, forward: bool) -> Option<(usize, usize)> {
        if self
            .pattern
            .as_ref()
            .is_none_or(|(compiled, _)| compiled != query)
        {
            let pattern = ModalPattern::new(&query.regex, query.ignore_case).ok()?;
            *self.pattern = Some((query.clone(), pattern));
        }
        let highlight = Deferred::Highlight(query.clone());
        if !self.deferred.contains(&highlight) {
            self.deferred.push(highlight);
        }
        let (_, pattern) = self.pattern.as_ref()?;
        self.doc.find_modal(pattern, from, forward)
    }

    fn mark(&self, name: char) -> Option<usize> {
        self.doc.modal_mark(name)
    }

    fn set_mark(&mut self, name: char, pos: usize) {
        self.doc.set_modal_mark(name, pos);
    }

    fn changes(&self) -> Vec<usize> {
        self.doc.modal_changes().to_vec()
    }

    fn record_change(&mut self, pos: usize) {
        self.doc.record_modal_change(pos);
    }

    fn record_jump(&mut self, _pos: usize) {
        if !self.deferred.contains(&Deferred::MarkJump) {
            self.deferred.push(Deferred::MarkJump);
        }
    }

    fn jump(&mut self, back: bool, count: usize) -> bool {
        // 目标可以在另一个文档中：按键处理结束之后由应用执行。
        self.deferred.push(Deferred::Jump(back, count));
        false
    }

    fn scroll(&mut self, scroll: Scroll) -> bool {
        match scroll {
            Scroll::Lines(lines) => {
                let before = self.doc.scroll_line;
                let target = (before + lines as f64).clamp(0.0, self.view.max_scroll.max(0.0));
                self.doc.scroll_line = target;
                self.refresh_visible();
                target != before
            }
            Scroll::Reveal { line, place } => {
                self.deferred.push(Deferred::Reveal(line, place));
                true
            }
            Scroll::Columns(columns) => {
                if self.doc.is_wrapping() {
                    return false;
                }
                let before = self.doc.scroll_x;
                self.doc.scroll_x = (before + columns as f32 * self.view.cell).max(0.0);
                self.doc.scroll_x != before
            }
        }
    }

    fn visible_cells(&self) -> Option<Range<usize>> {
        if self.doc.is_wrapping() {
            return None;
        }
        let first = (self.doc.scroll_x / self.view.cell.max(1.0)).ceil() as usize;
        Some(first..first + self.view.text_width)
    }

    fn scroll_margin(&self) -> usize {
        self.view.scroll_off
    }

    fn fold_at(&mut self, line: usize, fold: Fold) -> bool {
        let collapsed = self.doc.collapsed_fold_at(line).is_some();
        let command = match fold {
            Fold::Open => FoldCommand::Unfold,
            Fold::Close => FoldCommand::Fold,
            Fold::Toggle => FoldCommand::Toggle,
            Fold::OpenTree => FoldCommand::UnfoldRecursively,
            Fold::CloseTree => FoldCommand::FoldRecursively,
            Fold::ToggleTree if collapsed => FoldCommand::UnfoldRecursively,
            Fold::ToggleTree => FoldCommand::FoldRecursively,
            Fold::OpenAll => FoldCommand::UnfoldAll,
            Fold::CloseAll => FoldCommand::FoldAll,
            Fold::Reveal => {
                let pos = self.doc.selection.head;
                return self.doc.reveal(pos);
            }
        };
        // 折叠的变化连同滚动位置的保持与动画由应用执行。
        self.deferred.push(Deferred::Fold(command));
        false
    }

    fn fold_edge(&mut self, line: usize, forward: bool) -> Option<usize> {
        self.doc.fold_edge(line, forward, self.view.tab_width)
    }

    fn format_lines(&mut self, first: usize, last: usize) -> bool {
        self.deferred.push(Deferred::Format(first, last));
        true
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
        }
    }

    /// 是否高亮 Vim 层最近一次查找的匹配：查找条没有打开，且查找内容仍是该模式。
    pub(crate) fn vim_highlights(&self) -> bool {
        let Some(state) = &self.vim else {
            return false;
        };
        state.highlight
            && !self.find.open
            && self.search.regex
            && (state.pattern.as_ref()).is_some_and(|(query, _)| query.regex == self.search.query)
    }

    /// 返回输入条的位置：查找模式的输入条位于编辑区底部，其余位于顶部居中。
    pub(crate) fn prompt_bar(&self) -> Rect {
        let editor = self.layout.editor;
        match self.prompt_kind {
            PromptKind::VimSearch { .. } => {
                let bar = crate::layout::goto_bar(editor);
                let y = (editor.bottom() - bar.h - 8.0).max(editor.y);
                Rect::new(editor.x + 8.0, y, bar.w, bar.h)
            }
            _ => crate::layout::goto_bar(editor),
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
            scroll_off: self.config.vim_scroll_off.unwrap_or(0),
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
        let prompting = self.goto_input.is_some()
            && matches!(self.prompt_kind, PromptKind::VimSearch { .. })
            && self.focus == Focus::Prompt;
        if state.vim.prompt().is_some() {
            if prompting {
                let text = (self.goto_input.as_ref()).map(|field| field.text().to_owned());
                if state.previewed != text {
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
        if let Some(Prompt::Search { forward }) = prompt {
            self.vim_open_prompt(forward);
        }
        claimed
    }

    /// 打开查找模式的输入条；历史取自各窗口共用的查找历史。
    fn vim_open_prompt(&mut self, forward: bool) {
        let mut field = TextField::default();
        for entry in shared().search_history() {
            field.set_text(entry);
            field.remember();
        }
        field.set_text("");
        if let Some(state) = &mut self.vim {
            state.previewed = Some(String::new());
            state.highlight = true;
        }
        self.open_prompt(PromptKind::VimSearch { forward }, field);
    }

    /// 查找模式的输入条按下回车：以 `text` 为模式执行等待中的命令。
    pub(in crate::app) fn vim_prompt_submit(&mut self, text: &str) {
        let Some(mut state) = self.vim.take() else {
            return;
        };
        let lines = (self.editor.active()).map_or(0, |doc| doc.text().line_count());
        let result = self.vim_with(&mut state, |vim, shared, host| {
            vim.prompt_submit(shared, host, text);
        });
        state.previewed = None;
        self.vim = Some(state);
        if let Some(((), messages, deferred)) = result {
            self.vim_after(lines, messages, deferred);
        }
    }

    /// 放弃查找模式的输入：光标回到输入之前的位置。
    pub(in crate::app) fn vim_prompt_cancel(&mut self) {
        let Some(mut state) = self.vim.take() else {
            return;
        };
        self.vim_with(&mut state, |vim, _, host| vim.prompt_cancel(host));
        state.previewed = None;
        state.highlight = false;
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
            }
        }
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
                Message::NotFound => t!("editor-vim-not-found").to_owned(),
                Message::BadPattern(at) => {
                    t!("editor-vim-bad-pattern", column = (at + 1).to_string())
                }
                Message::NoSearch => t!("editor-vim-no-search").to_owned(),
                Message::NoMark(name) => t!("editor-vim-no-mark", name = name.to_string()),
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
        // 滚动命令已把所在行放到指定的位置，不再为光标调整滚动位置。
        if !revealed {
            self.ensure_caret_visible();
            self.vim_keep_margin();
        }
        self.reset_blink();
        self.request_redraw();
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
        let margin = self.config.vim_scroll_off.unwrap_or(0);
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
