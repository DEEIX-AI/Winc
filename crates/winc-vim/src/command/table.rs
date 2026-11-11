//! 命令表的内容，以及按模式建立的前缀树。

use std::sync::OnceLock;

use super::TextObject;
use super::{Action, Arg, Entry, Force, Kind, Mode, Modes, Motion, Operator, Shape, Support};
use crate::key::{Key, notation};

/// 创建一条没有参数、状态为 [`Support::Planned`] 的记录。
const fn entry(keys: &'static str, modes: Modes, kind: Kind) -> Entry {
    Entry {
        keys,
        modes,
        kind,
        arg: Arg::None,
        support: Support::Planned,
    }
}

/// 创建在普通、可视模式与等待动作时生效的动作。
const fn motion(keys: &'static str, motion: Motion) -> Entry {
    entry(keys, Modes::NVO, Kind::Motion(motion))
}

/// 创建在普通与可视模式下生效的操作符。
const fn operator(keys: &'static str, operator: Operator) -> Entry {
    entry(keys, Modes::NV, Kind::Operator(operator))
}

/// 创建在可视模式与等待动作时生效的文本对象。
const fn object(keys: &'static str, shape: Shape, around: bool) -> Entry {
    entry(
        keys,
        Modes::VO,
        Kind::TextObject(TextObject { shape, around }),
    )
}

/// 创建不带范围的命令。
const fn action(keys: &'static str, modes: Modes, action: Action) -> Entry {
    entry(keys, modes, Kind::Action(action))
}

impl Entry {
    /// 返回带一个字符参数的记录。
    const fn with_char(mut self) -> Self {
        self.arg = Arg::Char;
        self
    }

    /// 返回状态为 [`Support::Unsupported`] 的记录。
    const fn unsupported(mut self) -> Self {
        self.support = Support::Unsupported;
        self
    }
}

/// 全部记录。
///
/// 同一模式下任何记录的按键序列都不是另一条记录的前缀，按键序列写成规范的按键记法。
pub static ENTRIES: &[Entry] = &[
    // 前缀。
    entry("\"", Modes::NV, Kind::Register).with_char(),
    entry("v", Modes::O, Kind::Force(Force::Charwise)),
    entry("V", Modes::O, Kind::Force(Force::Linewise)),
    entry("<C-v>", Modes::O, Kind::Force(Force::Blockwise)),
    // 动作。
    motion("h", Motion::Left),
    motion("<Left>", Motion::Left),
    motion("<BS>", Motion::Left),
    motion("l", Motion::Right),
    motion("<Right>", Motion::Right),
    motion("<Space>", Motion::Right),
    motion("k", Motion::Up),
    motion("<Up>", Motion::Up),
    motion("j", Motion::Down),
    motion("<Down>", Motion::Down),
    motion("w", Motion::WordForward),
    motion("b", Motion::WordBackward),
    motion("e", Motion::WordEnd),
    motion("ge", Motion::WordEndBackward),
    motion("0", Motion::LineStart),
    motion("<Home>", Motion::LineStart),
    motion("^", Motion::FirstNonBlank),
    motion("$", Motion::LineEnd),
    motion("<End>", Motion::LineEnd),
    motion("gg", Motion::FirstLine),
    motion("G", Motion::LastLine),
    motion("f", Motion::FindForward).with_char(),
    motion("F", Motion::FindBackward).with_char(),
    motion("t", Motion::TillForward).with_char(),
    motion("T", Motion::TillBackward).with_char(),
    motion(";", Motion::RepeatFind),
    motion(",", Motion::RepeatFindReverse),
    // 操作符。
    operator("d", Operator::Delete),
    operator("c", Operator::Change),
    operator("y", Operator::Yank),
    operator(">", Operator::Indent),
    operator("<lt>", Operator::Outdent),
    operator("g~", Operator::ToggleCase),
    operator("gu", Operator::Lowercase),
    operator("gU", Operator::Uppercase),
    operator("gq", Operator::Format).unsupported(),
    // 可视模式下直接作用于选区的操作符。
    entry("x", Modes::V, Kind::Operator(Operator::Delete)),
    entry("<Del>", Modes::V, Kind::Operator(Operator::Delete)),
    entry("u", Modes::V, Kind::Operator(Operator::Lowercase)),
    entry("U", Modes::V, Kind::Operator(Operator::Uppercase)),
    entry("~", Modes::V, Kind::Operator(Operator::ToggleCase)),
    // 文本对象。
    object("iw", Shape::Word, false),
    object("aw", Shape::Word, true),
    object("i(", Shape::Paren, false),
    object("i)", Shape::Paren, false),
    object("ib", Shape::Paren, false),
    object("a(", Shape::Paren, true),
    object("a)", Shape::Paren, true),
    object("ab", Shape::Paren, true),
    object("i\"", Shape::DoubleQuote, false),
    object("a\"", Shape::DoubleQuote, true),
    // 不带范围的命令。
    action("x", Modes::N, Action::DeleteChar),
    action("<Del>", Modes::N, Action::DeleteChar),
    action("X", Modes::N, Action::DeleteCharBefore),
    action("D", Modes::N, Action::DeleteToLineEnd),
    action("C", Modes::N, Action::ChangeToLineEnd),
    action("Y", Modes::N, Action::YankLine),
    action("p", Modes::NV, Action::PasteAfter),
    action("P", Modes::NV, Action::PasteBefore),
    action("r", Modes::NV, Action::ReplaceChar).with_char(),
    action("u", Modes::N, Action::Undo),
    action("<C-r>", Modes::N, Action::Redo),
    action(".", Modes::N, Action::Repeat),
    action("i", Modes::N, Action::Insert),
    action("a", Modes::N, Action::Append),
    action("v", Modes::NV, Action::Visual),
    action("V", Modes::NV, Action::VisualLine),
    action("<C-v>", Modes::NV, Action::VisualBlock),
    action("<C-q>", Modes::NV, Action::VisualBlock),
    action("m", Modes::NV, Action::SetMark).with_char(),
    action("<Esc>", Modes::V.with(Modes::I), Action::Escape),
    action("<C-r>", Modes::I, Action::InsertRegister).with_char(),
    action("Q", Modes::NV, Action::ExMode).unsupported(),
    action("K", Modes::NV, Action::Keyword).unsupported(),
    action("U", Modes::N, Action::UndoLine).unsupported(),
];

/// 前缀树的根节点。
pub(crate) const ROOT: u16 = 0;

/// 前缀树的一个节点。
#[derive(Debug, Default)]
struct Node {
    /// 子节点：按键与节点下标，按按键排序。
    children: Vec<(Key, u16)>,
    /// 按键序列在此结束的记录在 [`ENTRIES`] 中的下标；有记录的节点没有子节点。
    entry: Option<u16>,
}

/// 一步查找的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// 按键序列与一条记录完全匹配；值为该记录在 [`ENTRIES`] 中的下标。
    Entry(u16),
    /// 按键序列是若干记录的前缀；值为到达的节点，用于继续查找。
    Prefix(u16),
    /// 没有记录以该按键序列开头。
    None,
}

/// 按模式建立的前缀树。首次使用时由 [`ENTRIES`] 建立，之后只读。
#[derive(Debug)]
pub(crate) struct Table {
    /// 各模式的前缀树，下标为 [`Mode::index`]；每棵树的 0 号节点是根。
    tries: [Vec<Node>; Mode::ALL.len()],
    /// 各记录解析后的按键序列，下标与 [`ENTRIES`] 相同。
    keys: Vec<Vec<Key>>,
}

impl Table {
    /// 返回进程内唯一的命令表，首次调用时建立。
    pub(crate) fn get() -> &'static Self {
        static TABLE: OnceLock<Table> = OnceLock::new();
        TABLE.get_or_init(|| Self::build().0)
    }

    /// 由 [`ENTRIES`] 建立前缀树；同时返回未能加入的记录（模式与下标），正确的命令表中为空。
    ///
    /// 普通模式的操作符同时加入等待动作的前缀树，用于识别叠写形式。
    pub(super) fn build() -> (Self, Vec<(Mode, usize)>) {
        let keys: Vec<Vec<Key>> = ENTRIES.iter().map(|e| notation::parse(e.keys)).collect();
        let mut tries = Mode::ALL.map(|_| vec![Node::default()]);
        let mut rejected = Vec::new();
        for (index, (entry, sequence)) in ENTRIES.iter().zip(&keys).enumerate() {
            for mode in Mode::ALL {
                let doubled = mode == Mode::OperatorPending
                    && matches!(entry.kind, Kind::Operator(_))
                    && entry.modes.contains(Mode::Normal);
                if !entry.modes.contains(mode) && !doubled {
                    continue;
                }
                if !insert(&mut tries[mode.index()], sequence, index) {
                    rejected.push((mode, index));
                }
            }
        }
        (Self { tries, keys }, rejected)
    }

    /// 从 `mode` 的前缀树的节点 `node` 出发，按 `key` 前进一步。
    pub(crate) fn step(&self, mode: Mode, node: u16, key: Key) -> Lookup {
        let trie = &self.tries[mode.index()];
        let child = trie.get(usize::from(node)).and_then(|node| {
            let at = node.children.binary_search_by(|(k, _)| k.cmp(&key)).ok()?;
            node.children.get(at).map(|(_, child)| *child)
        });
        match child {
            None => Lookup::None,
            Some(child) => match trie.get(usize::from(child)).and_then(|n| n.entry) {
                Some(entry) => Lookup::Entry(entry),
                None => Lookup::Prefix(child),
            },
        }
    }

    /// 返回下标为 `index` 的记录；下标越界时返回 `None`。
    pub(crate) fn entry(&self, index: u16) -> Option<&'static Entry> {
        ENTRIES.get(usize::from(index))
    }

    /// 返回下标为 `index` 的记录的按键序列；下标越界时为空。
    pub(crate) fn keys(&self, index: u16) -> &[Key] {
        self.keys.get(usize::from(index)).map_or(&[], Vec::as_slice)
    }
}

/// 把按键序列 `keys` 对应的记录 `entry` 加入前缀树 `trie`，返回是否成功。
///
/// 按键序列为空、与已有记录相同，或与已有记录互为前缀时不加入。
fn insert(trie: &mut Vec<Node>, keys: &[Key], entry: usize) -> bool {
    let Ok(entry) = u16::try_from(entry) else {
        return false;
    };
    let mut at = usize::from(ROOT);
    for key in keys {
        if trie[at].entry.is_some() {
            return false;
        }
        at = match trie[at].children.binary_search_by(|(k, _)| k.cmp(key)) {
            Ok(i) => usize::from(trie[at].children[i].1),
            Err(i) => {
                let Ok(child) = u16::try_from(trie.len()) else {
                    return false;
                };
                trie.push(Node::default());
                trie[at].children.insert(i, (*key, child));
                usize::from(child)
            }
        };
    }
    let node = &mut trie[at];
    if keys.is_empty() || node.entry.is_some() || !node.children.is_empty() {
        return false;
    }
    node.entry = Some(entry);
    true
}
