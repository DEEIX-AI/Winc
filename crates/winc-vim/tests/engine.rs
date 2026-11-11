//! 状态机的手写测试：模式的切换、未完成的按键、撤销分组、只读文档、折叠、剪贴板、窗口与
//! 显示行的动作等录制覆盖不到的行为。

mod common;

use winc_core::{Lines, Selection};
use winc_vim::{Clipboard, Indentation, Input, Key, Message, Mode, Named, Shared, Vim, VisualKind};
use winc_vim::{Host, notation};

use common::{TestHost, feed};

/// 一次测试的状态：宿主、窗口的状态与共用的状态。
struct Session {
    host: TestHost,
    vim: Vim,
    shared: Shared,
}

impl Session {
    /// 创建文本为 `text`（各行以 `\n` 连接，末尾另加换行符）、光标在开头的状态。
    fn new(text: &str) -> Self {
        Self {
            host: TestHost::new(&format!("{text}\n")),
            vim: Vim::default(),
            shared: Shared::default(),
        }
    }

    /// 输入按键记法 `keys`。
    fn keys(&mut self, keys: &str) -> &mut Self {
        feed(&mut self.vim, &mut self.shared, &mut self.host, keys);
        self
    }

    /// 返回去掉末尾换行符的文本。
    fn text(&self) -> String {
        let text = self.host.text();
        text.strip_suffix('\n').unwrap_or(&text).to_owned()
    }

    /// 返回光标的行号与列号。
    fn caret(&self) -> (usize, usize) {
        self.host.rope.pos_to_line_col(self.vim.caret(&self.host))
    }

    /// 返回未完成的按键的记法。
    fn pending(&self) -> String {
        notation::write(self.vim.pending())
    }
}

#[test]
fn modes_follow_the_commands() {
    let mut s = Session::new("one two\nthree");
    assert_eq!(s.vim.mode(), Mode::Normal);
    s.keys("d");
    assert!(s.vim.is_operator_pending());
    assert_eq!((s.vim.mode(), s.pending().as_str()), (Mode::Normal, "d"));
    s.keys("<Esc>");
    assert!(!s.vim.is_operator_pending());
    assert_eq!(s.pending(), "");

    s.keys("i");
    assert_eq!(s.vim.mode(), Mode::Insert);
    s.keys("ab<Esc>");
    assert_eq!(
        (s.vim.mode(), s.text().as_str()),
        (Mode::Normal, "abone two\nthree")
    );

    s.keys("v");
    assert_eq!(s.vim.visual_kind(), Some(VisualKind::Char));
    s.keys("V");
    assert_eq!(
        (s.vim.mode(), s.vim.visual_kind()),
        (Mode::Visual, Some(VisualKind::Line))
    );
    s.keys("V");
    assert_eq!((s.vim.mode(), s.vim.visual_kind()), (Mode::Normal, None));
    s.keys("vcX");
    assert_eq!(s.vim.mode(), Mode::Insert);
    s.keys("<Esc>2\"a3");
    assert_eq!(s.pending(), "2\"a3");
    s.vim.cancel_pending();
    assert_eq!(s.pending(), "");
}

#[test]
fn keys_without_a_meaning_are_swallowed_or_passed_on() {
    let mut s = Session::new("abc");
    let key = |s: &mut Session, text: &str| {
        let key = notation::parse(text)[0];
        s.vim.key(&mut s.shared, &mut s.host, key)
    };
    // 普通模式：没有定义的可见字符不交给宿主，功能键与带修饰键的组合交给宿主。
    assert!(key(&mut s, "\\"));
    assert!(key(&mut s, "中"));
    assert!(!key(&mut s, "<F5>"));
    assert!(!key(&mut s, "<C-S-p>"));
    assert!(!key(&mut s, "<PageDown>"));
    assert!(!key(&mut s, "<Esc>"));
    assert_eq!(s.text(), "abc");
    // 有未完成的按键时 `<Esc>` 由 Vim 层处理。
    assert!(key(&mut s, "2"));
    assert!(key(&mut s, "<Esc>"));
    // 插入模式：只处理 `<Esc>`。
    assert!(key(&mut s, "i"));
    assert!(!key(&mut s, "x"));
    assert!(!key(&mut s, "<CR>"));
    assert!(!key(&mut s, "<C-r>"));
    assert!(key(&mut s, "<Esc>"));
    assert_eq!(s.vim.mode(), Mode::Normal);
}

#[test]
fn unsupported_commands_only_show_a_message() {
    let mut s = Session::new("one (two) three");
    for keys in [
        "Q", "K", "U", "diw", "ci(", ":", "/", "n", "ma", "qa", "@a", "<C-v>", "zz", "gqq", "==",
        "=j",
    ] {
        s.host.messages.clear();
        s.keys(keys);
        assert_eq!(s.host.messages, [Message::Unsupported], "{keys}");
        assert_eq!(
            (s.text().as_str(), s.vim.mode()),
            ("one (two) three", Mode::Normal),
            "{keys}"
        );
        assert_eq!(s.pending(), "", "{keys}");
    }
}

#[test]
fn read_only_documents_reject_changes_and_keep_the_mode() {
    let mut s = Session::new("one two\nthree");
    s.host.read_only = true;
    for keys in [
        "x", "dd", "cw", "i", "A", "o", "p", "J", "rx", "~", ">>", "g~~", "vd", "Vc",
    ] {
        s.host.messages.clear();
        s.keys(keys);
        assert_eq!(s.host.messages, [Message::ReadOnly], "{keys}");
        assert_eq!(s.text(), "one two\nthree", "{keys}");
        assert_ne!(s.vim.mode(), Mode::Insert, "{keys}");
        s.keys("<Esc>");
    }
    // 移动与复制仍然可用。
    s.host.messages.clear();
    s.keys("wyiw");
    s.keys("<Esc>yw");
    assert_eq!(s.caret(), (0, 4));
    assert_eq!(s.shared.registers.get(None).unwrap().text, "two");
}

#[test]
fn a_command_and_its_insert_are_one_undo_unit() {
    let mut s = Session::new("one two three");
    s.keys("cwfoo bar<Esc>");
    assert_eq!(s.text(), "foo bar two three");
    assert_eq!(s.host.groups, 0);
    s.keys("u");
    assert_eq!((s.text().as_str(), s.caret()), ("one two three", (0, 0)));
    assert!(!s.host.undo(), "一次撤销即还原");

    // 带次数的命令、点重复各是一个单元。
    s.keys("3x");
    s.keys("w.");
    assert_eq!(s.text(), "  three");
    s.keys("u");
    assert_eq!(s.text(), " two three");
    s.keys("u");
    assert_eq!(s.text(), "one two three");

    // 重复输入的插入是一个单元。
    s.keys("3oab<Esc>u");
    assert_eq!(s.text(), "one two three");
}

#[test]
fn moving_the_caret_while_inserting_starts_a_new_unit_and_a_new_record() {
    let mut s = Session::new("....");
    s.keys("iab");
    assert_eq!(s.host.groups, 1);
    // 宿主移动了光标（方向键、鼠标）。
    s.host.selection = Selection::point(4);
    s.vim.insert_moved(&mut s.host);
    s.keys("cd<Esc>");
    assert_eq!((s.text().as_str(), s.host.groups), ("ab..cd..", 0));
    // 点重复只重放移动之后的输入。
    s.keys("$.");
    assert_eq!(s.text(), "ab..cd.cd.");
    s.keys("u");
    s.keys("u");
    assert_eq!(s.text(), "ab....");
    s.keys("u");
    assert_eq!(s.text(), "....");
}

#[test]
fn inputs_made_by_the_host_are_replayed() {
    let mut s = Session::new("x\ny");
    s.keys("A");
    for input in [
        Input::Text("ab".into()),
        Input::Backspace { word: false },
        Input::Newline,
        Input::Paste("p q".into()),
    ] {
        s.host.input(&input);
        s.vim.typed(input);
    }
    s.keys("<Esc>");
    assert_eq!(s.text(), "xa\np q\ny");
    s.keys("j.");
    assert_eq!(s.text(), "xa\np q\nya\np q");
    assert_eq!(s.shared.registers.get(Some('.')).unwrap().text, "ab\np q");
}

#[test]
fn selections_made_outside_enter_and_leave_visual_mode() {
    let mut s = Session::new("one two\nthree");
    // 宿主选中了 `ne t`（鼠标拖动）。
    s.host.selection = Selection::new(1, 5);
    s.vim.sync(&mut s.host);
    assert_eq!(
        (s.vim.mode(), s.vim.visual_kind()),
        (Mode::Visual, Some(VisualKind::Char))
    );
    assert_eq!(s.caret(), (0, 4));
    s.keys("d");
    assert_eq!(
        (s.text().as_str(), s.vim.mode()),
        ("owo\nthree", Mode::Normal)
    );

    // 反向的选区：光标在前。
    s.host.selection = Selection::new(3, 1);
    s.vim.sync(&mut s.host);
    assert_eq!(s.caret(), (0, 1));
    s.keys("y");
    assert_eq!(s.shared.registers.get(None).unwrap().text, "wo");

    // 选区被清空时回到普通模式；光标落在行尾之后时退到最后一个字符。
    s.host.selection = Selection::new(0, 2);
    s.vim.sync(&mut s.host);
    assert_eq!(s.vim.mode(), Mode::Visual);
    s.host.selection = Selection::point(3);
    s.vim.sync(&mut s.host);
    assert_eq!((s.vim.mode(), s.caret()), (Mode::Normal, (0, 2)));
    assert_eq!(s.host.selection, Selection::point(2));

    // 插入模式下选区不改变模式。
    s.keys("i");
    s.host.selection = Selection::new(0, 2);
    s.vim.sync(&mut s.host);
    assert_eq!(s.vim.mode(), Mode::Insert);
}

#[test]
fn visual_selections_cover_both_ends() {
    let mut s = Session::new("one two\nthree\nfour");
    s.keys("lvl");
    assert_eq!(s.host.selection, Selection::new(1, 3));
    s.keys("o");
    assert_eq!(
        (s.host.selection, s.caret()),
        (Selection::new(3, 1), (0, 1))
    );
    s.keys("<Esc>");
    assert_eq!(s.host.selection, Selection::point(1));

    // 按行的选区覆盖整行，含行尾的换行符。
    s.keys("Vj");
    assert_eq!(s.host.selection, Selection::new(0, 14));
    assert_eq!(s.caret(), (1, 1));
    s.keys("<Esc>jVk");
    assert_eq!(s.host.selection, Selection::new(19, 8));
    // `$` 之后光标在行尾，选区包含换行符。
    s.keys("<Esc>ggv$");
    assert_eq!(
        (s.host.selection, s.caret()),
        (Selection::new(0, 8), (0, 7))
    );
}

#[test]
fn folded_lines_move_and_operate_as_one() {
    let mut s = Session::new("a\nb\nc\nd\ne");
    s.host.folds = vec![(1, 3)];
    s.keys("j");
    assert_eq!(s.caret(), (1, 0));
    s.keys("j");
    assert_eq!(s.caret(), (4, 0));
    s.keys("k");
    assert_eq!(s.caret(), (1, 0));
    s.keys("yy");
    assert_eq!(s.shared.registers.get(None).unwrap().text, "b\nc\nd");
    s.keys("ggjp");
    assert_eq!(s.text(), "a\nb\nc\nd\nb\nc\nd\ne");
    s.keys("u");
    s.keys("ggjdd");
    assert_eq!(s.text(), "a\ne");
}

#[test]
fn the_clipboard_follows_the_plus_register_and_the_sync_mode() {
    let mut s = Session::new("one\ntwo");
    s.keys("\"+yy");
    assert_eq!(s.host.clipboard.as_deref(), Some("one\n"));
    // 读回相同的文本时保留按行的类型；其他程序写入的文本按内容判断。
    s.keys("\"+p");
    assert_eq!(s.text(), "one\none\ntwo");
    s.host.clipboard = Some("xy".into());
    s.keys("\"*P");
    assert_eq!(s.text(), "one\nxyone\ntwo");
    s.host.clipboard = None;
    s.host.messages.clear();
    s.keys("\"+p");
    assert_eq!(s.host.messages, [Message::EmptyRegister('+')]);

    // 只同步复制：删除不覆盖剪贴板。
    s.shared.clipboard = Clipboard::Yank;
    s.keys("yiw");
    s.keys("<Esc>yl");
    assert_eq!(s.host.clipboard.as_deref(), Some("y"));
    s.keys("dd");
    assert_eq!(s.host.clipboard.as_deref(), Some("y"));
    s.keys("\"ayl");
    assert_eq!(s.host.clipboard.as_deref(), Some("y"));

    // 全部同步：删除写入剪贴板，粘贴读取剪贴板。
    s.shared.clipboard = Clipboard::Always;
    s.keys("x");
    assert_eq!(s.host.clipboard.as_deref(), Some("t"));
    s.host.clipboard = Some("from outside".into());
    s.keys("P");
    assert_eq!(s.text(), "one\nfrom outsidewo");
}

#[test]
fn registers_report_missing_content_and_refuse_writes() {
    let mut s = Session::new("one");
    s.keys("\"ap");
    s.keys("p");
    s.keys("\"%p");
    assert_eq!(
        s.host.messages,
        [
            Message::EmptyRegister('a'),
            Message::EmptyRegister('"'),
            Message::EmptyRegister('%'),
        ]
    );
    s.host.messages.clear();
    s.keys("\".yy");
    s.keys("\":dd");
    assert_eq!(
        s.host.messages,
        [
            Message::ReadOnlyRegister('.'),
            Message::ReadOnlyRegister(':')
        ]
    );
    assert_eq!(s.text(), "one");
    // 黑洞寄存器不保留内容，也不改动其他寄存器。
    s.keys("yy\"_dd");
    assert_eq!(s.text(), "");
    assert_eq!(s.shared.registers.get(None).unwrap().text, "one");
}

#[test]
fn counts_that_would_generate_too_much_text_are_refused() {
    let mut s = Session::new("one");
    s.keys("yy99999999p");
    assert_eq!(s.host.messages, [Message::TooLarge]);
    assert_eq!(s.text(), "one");
    s.host.messages.clear();
    s.keys("V999999999>");
    assert_eq!(s.host.messages, [Message::TooLarge]);
    // 重复输入超过上限时只执行一次；空的输入不重复。
    s.keys("<Esc>999999999ix<Esc>");
    assert_eq!(s.text(), "xone");
    s.keys("999999999i<Esc>");
    s.keys("999999999o<Esc>");
    assert_eq!(s.text(), "xone\n");
    // 次数极大的动作在有限的步数内结束。
    s.keys("999999999w999999999b999999999l999999999h999999999j999999999k");
    assert_eq!(s.caret(), (0, 0));
    s.keys("999999999x");
    assert_eq!(s.text(), "\n");
}

#[test]
fn the_comment_operator_goes_to_the_host() {
    let mut s = Session::new("a\nb\nc\nd");
    s.keys("gcc");
    s.keys("jgcj");
    s.keys("Vjgc");
    assert_eq!(s.host.commented, [(0, 0), (1, 2), (1, 2)]);
    s.keys("gg3gcc");
    assert_eq!(s.host.commented.last(), Some(&(0, 2)));
}

#[test]
fn window_motions_use_the_visible_lines() {
    let text: Vec<String> = (0..30).map(|n| format!("  line {n}")).collect();
    let mut s = Session::new(&text.join("\n"));
    s.host.visible = Some(10..20);
    s.keys("H");
    assert_eq!(s.caret(), (10, 2));
    s.keys("L");
    assert_eq!(s.caret(), (19, 2));
    s.keys("M");
    assert_eq!(s.caret(), (14, 2));
    s.keys("3H");
    assert_eq!(s.caret(), (12, 2));
    s.keys("3L");
    assert_eq!(s.caret(), (17, 2));
    // 窗口超出文本末尾时以最后一行为底。
    s.host.visible = Some(25..40);
    s.keys("L");
    assert_eq!(s.caret(), (29, 2));
    s.keys("M");
    assert_eq!(s.caret(), (27, 2));
    s.keys("dH");
    assert_eq!(s.text().lines().count(), 27);
}

#[test]
fn display_line_motions_follow_the_wrapping() {
    // 每 10 个字符一个显示行：第一行占三个显示行。
    let mut s = Session::new("0123456789abcdefghijABCDE\nxy\nlast line here");
    s.host.wrap = Some(10);
    s.keys("3l");
    s.keys("gj");
    assert_eq!(s.caret(), (0, 13));
    s.keys("gj");
    assert_eq!(s.caret(), (0, 23));
    s.keys("g0");
    assert_eq!(s.caret(), (0, 20));
    s.keys("g$");
    assert_eq!(s.caret(), (0, 24));
    s.keys("gk");
    s.keys("g$");
    assert_eq!(s.caret(), (0, 19));
    s.keys("gm");
    assert_eq!(s.caret(), (0, 15));
    s.keys("g0dg$");
    assert_eq!(s.text(), "0123456789ABCDE\nxy\nlast line here");
    // `j`、`k` 仍按文本行移动。
    s.keys("ggj");
    assert_eq!(s.caret(), (1, 0));
    s.keys("3gj");
    assert_eq!(s.caret(), (2, 10));
}

#[test]
fn hosts_that_keep_indentation_get_it_on_new_and_changed_lines() {
    let mut s = Session::new("    one\n    two");
    s.host.keeps_indent = true;
    s.host.indentation = Indentation {
        width: 4,
        tabs: false,
    };
    s.keys("ccx<Esc>");
    assert_eq!(s.text(), "    x\n    two");
    s.keys("jOy<Esc>");
    assert_eq!(s.text(), "    x\n    y\n    two");
    s.keys(">>");
    assert_eq!(s.text(), "    x\n        y\n    two");
    s.keys("Vj<lt>");
    assert_eq!(s.text(), "    x\n    y\ntwo");
}

#[test]
fn insert_returns_to_the_last_insert_position() {
    let mut s = Session::new("one two\nthree");
    s.keys("wiab<Esc>");
    s.keys("G$giXY<Esc>");
    assert_eq!(s.text(), "one abXYtwo\nthree");
    // 没有上一次插入时在光标处插入。
    let mut fresh = Session::new("abc");
    fresh.keys("lgiZ<Esc>");
    assert_eq!(fresh.text(), "aZbc");
}

#[test]
fn resetting_ends_the_open_group_and_returns_to_normal_mode() {
    let mut s = Session::new("abc");
    s.keys("ix");
    assert_eq!((s.vim.mode(), s.host.groups), (Mode::Insert, 1));
    s.vim.reset(&mut s.host);
    assert_eq!((s.vim.mode(), s.host.groups), (Mode::Normal, 0));
    s.keys("vl");
    s.vim.reset(&mut s.host);
    assert_eq!((s.vim.mode(), s.pending().as_str()), (Mode::Normal, ""));
    let key = Key::named(Named::Esc);
    assert!(!s.vim.key(&mut s.shared, &mut s.host, key));
}
