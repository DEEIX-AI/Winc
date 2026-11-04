//! 终端里的查找（⌘F）：在回滚缓冲加屏幕里找文字。
//!
//! 打开查找条时向辅助进程要整个回滚缓冲的文字（辅助进程不带正则库，查找在主程序这边做），
//! 只留文字、不留样式，关掉查找条时释放。查找的词每改一次就从最下面一行往上重新找，每帧
//! 只找一个时间片（[`SLICE`]），回滚缓冲很长时输入框也不阻塞；默认不区分大小写，查找的词
//! 里有大写字母时改为区分。

use std::time::{Duration, Instant};

use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use winc_pty::protocol::Line;

use super::tab::{Point, Selection, Unit, find_in_line};
use super::{App, Focus, Request};
use crate::field::TextField;

/// 每次查找最多连续占用的时间（8 ms），与编辑区查找大文件的时间片相同。
const SLICE: Duration = Duration::from_millis(8);

/// 查找条的状态。
#[derive(Default)]
pub(crate) struct TermFind {
    /// 查找的词。
    pub field: TextField,
    /// 在哪个标签里找。
    pub tab: u16,
    /// 打开时取回的回滚缓冲各行的文字，分几段回来：（第一行的行号，各行），按位置排好。
    chunks: Vec<(u64, Vec<String>)>,
    /// 回滚缓冲取回来了没有（没回来时先只找屏幕）。
    pub loaded: bool,
    /// 所有匹配，按位置从上到下：（起点，终点）。
    pub matches: Vec<(Point, Point)>,
    /// 当前的那一处。
    pub current: Option<usize>,
    /// 还没找完时，下一个时间片从这一行的上一行往上找。
    scan: Option<u64>,
}

impl App {
    /// 打开查找条（⌘F）；已经开着时把焦点放回输入框、全选。
    pub(in crate::app) fn open_terminal_find(&mut self) {
        let t = &mut self.terminal;
        let Some(tab) = t.tabs.get(t.active) else {
            return;
        };
        let (id, history) = (tab.id, tab.history);
        if t.find.as_ref().is_none_or(|f| f.tab != id) {
            t.find = Some(TermFind {
                tab: id,
                ..TermFind::default()
            });
            t.request(id, history, Request::Find);
            // 回滚缓冲是空的：没有要等的。
            if let Some(find) = &mut t.find {
                find.loaded = history.0 >= history.1;
            }
        }
        if let Some(find) = &mut t.find {
            find.field.select_all();
        }
        self.focus = Focus::TerminalFind;
        self.terminal_find_changed();
        self.request_redraw();
    }

    /// 关掉查找条，焦点回终端；保留选中的匹配（可以直接 ⌘C）。
    pub(in crate::app) fn close_terminal_find(&mut self) {
        self.terminal.find = None;
        // 还没回来的各段不再使用：马上再 ⌘F 时混进新的查找，每处匹配算两遍。
        self.terminal
            .requests
            .retain(|r| r.2 != super::Request::Find);
        self.focus = Focus::Terminal;
        self.request_redraw();
    }

    /// 回滚缓冲的一段文字取回来了（`done`：最后一段）。
    pub(in crate::app) fn terminal_find_loaded(
        &mut self,
        tab: u16,
        start: u64,
        lines: Vec<Line>,
        done: bool,
    ) {
        if let Some(find) = self.terminal.find.as_mut().filter(|f| f.tab == tab) {
            let lines = lines.into_iter().map(|line| line.text).collect();
            find.chunks.push((start, lines));
            find.loaded = done;
            if done {
                self.terminal_find_changed();
            }
        }
    }

    /// 标签的行号全部重来了（改尺寸重排、⌘K）：查找要重新取回滚缓冲，F8 记的位置作废。
    pub(in crate::app) fn terminal_renumbered(&mut self, id: u16) {
        let t = &mut self.terminal;
        if t.errors.as_ref().is_some_and(|e| e.tab == id) {
            t.errors = None;
        }
        let Some(history) = t
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.history)
        else {
            return;
        };
        if let Some(find) = t.find.as_mut().filter(|f| f.tab == id) {
            find.chunks.clear();
            find.loaded = history.0 >= history.1;
            t.requests
                .retain(|&(_, tab, purpose)| (tab, purpose) != (id, Request::Find));
            t.request(id, history, Request::Find);
            self.terminal_find_changed();
        }
    }

    /// 查找的词变了（或者文字刚取回来）：清掉旧的匹配，从最下面一行往上重新找。
    ///
    /// 这里先找一个时间片，没找完的部分在之后的帧里由 [`App::continue_terminal_find`] 继续。
    pub(in crate::app) fn terminal_find_changed(&mut self) {
        self.restart_terminal_find();
        self.continue_terminal_find(SLICE);
    }

    /// 清掉旧的匹配，把查找的位置放回最下面一行之后，不开始找。
    pub(in crate::app) fn restart_terminal_find(&mut self) {
        let t = &mut self.terminal;
        let Some(find) = &mut t.find else { return };
        let end = t
            .tabs
            .iter()
            .find(|tab| tab.id == find.tab)
            .map(|tab| tab.history.1 + tab.rows.len() as u64);
        find.matches.clear();
        find.current = None;
        find.scan = end.filter(|_| !find.field.text().is_empty());
        self.reveal_terminal_match();
    }

    /// 画完一帧以后再找一个时间片，返回是否还没找完。
    pub(in crate::app) fn terminal_find_tick(&mut self) -> bool {
        self.continue_terminal_find(SLICE);
        self.terminal
            .find
            .as_ref()
            .is_some_and(|f| f.scan.is_some())
    }

    /// 往上继续找，最多用 `budget`；第一次找到匹配时选中最下面的那一处。
    pub(in crate::app) fn continue_terminal_find(&mut self, budget: Duration) {
        let t = &mut self.terminal;
        let Some(find) = &mut t.find else { return };
        let Some(mut next) = find.scan else { return };
        let Some(tab) = t.tabs.iter().find(|tab| tab.id == find.tab) else {
            find.scan = None;
            return;
        };
        let query = find.field.text();
        let case_sensitive = query.chars().any(char::is_uppercase);
        let fetched = find
            .chunks
            .last()
            .map_or(tab.history.0, |(start, lines)| start + lines.len() as u64);
        // 第 `number` 行的文字：屏幕上的行、取回来以后又滚进回滚缓冲的行（取回的那份里没有，
        // 用标签自己缓存的）、取回的回滚缓冲，依次查。
        let text = |number: u64| -> Option<&str> {
            if number >= tab.history.1 {
                let row = usize::try_from(number - tab.history.1).ok()?;
                return tab.rows.get(row).map(|line| line.text.as_str());
            }
            if number >= fetched {
                return tab.line(number).map(|line| line.text.as_str());
            }
            let chunk = find.chunks.partition_point(|&(start, _)| start <= number);
            let (start, lines) = find.chunks.get(chunk.checked_sub(1)?)?;
            let index = usize::try_from(number - start).ok()?;
            lines.get(index).map(String::as_str)
        };
        // 从下往上找到的匹配，最后倒过来接在已有的匹配前面。
        let mut found = Vec::new();
        let started = Instant::now();
        while next > tab.history.0 {
            next -= 1;
            if let Some(line) = text(next) {
                let point = |col| Point { line: next, col };
                let hits = find_in_line(line, query, case_sensitive);
                found.extend(hits.into_iter().rev().map(|(a, b)| (point(a), point(b))));
            }
            if started.elapsed() >= budget {
                break;
            }
        }
        find.scan = (next > tab.history.0).then_some(next);
        found.reverse();
        let added = found.len();
        find.matches.splice(0..0, found);
        let first = find.current.is_none() && !find.matches.is_empty();
        find.current = match find.current {
            Some(i) => Some(i + added),
            None => find.matches.len().checked_sub(1),
        };
        if first {
            self.reveal_terminal_match();
        }
    }

    /// 换到上一处（`older`，往上）或下一处匹配，到头了绕回去。
    pub(in crate::app) fn terminal_find_step(&mut self, older: bool) {
        let Some(find) = &mut self.terminal.find else {
            return;
        };
        let n = find.matches.len();
        if n == 0 {
            return;
        }
        find.current = Some(match find.current {
            Some(i) if older => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None => n - 1,
        });
        self.reveal_terminal_match();
    }

    /// 选中当前的匹配，翻到能看见它的地方。
    fn reveal_terminal_match(&mut self) {
        let t = &mut self.terminal;
        let Some(find) = &t.find else { return };
        let Some(tab) = t.tabs.iter_mut().find(|tab| tab.id == find.tab) else {
            return;
        };
        let Some(&(start, end)) = find.current.and_then(|i| find.matches.get(i)) else {
            tab.selection = None;
            self.request_redraw();
            return;
        };
        tab.selection = Some(Selection {
            anchor: start,
            head: end,
            unit: Unit::Char,
        });
        tab.reveal(start.line);
        self.fetch_terminal_history();
        self.request_redraw();
    }

    /// 焦点在终端的查找条里时的按键。
    pub(in crate::app) fn on_key_terminal_find(&mut self, event: &KeyEvent, base: &str, cmd: bool) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_terminal_find(),
            Key::Named(NamedKey::Enter) => {
                let older = !self.modifiers.shift_key();
                self.terminal_find_step(older);
            }
            _ => self.on_key_field(event, base, cmd),
        }
    }
}
