//! 补全会话的测试：触发检测、防抖、过期响应、过滤与 Esc 后的抑制。

use super::*;

/// 在 `text` 的光标位于 `caret`（字符偏移）处的文档上下文。
fn context_at(text: &str, caret: usize, revision: u64) -> Option<Context> {
    let mut doc = Document::from_text(DocumentId(1), text);
    doc.revision = revision;
    doc.selection = winc_core::Selection::point(caret);
    Completion::context(&doc)
}

fn item(label: &str) -> CompletionItem {
    CompletionItem {
        label: label.into(),
        kind: Some(3),
        sort_text: None,
        filter_text: None,
        insert_text: None,
        text_edit: None,
        additional_text_edits: Vec::new(),
    }
}

/// 让会话进入「已看到过 rev1」的状态，返回观察 rev2 用的上下文。
fn trigger(c: &mut Completion, now: Instant) -> Context {
    let first = Context {
        doc: DocumentId(1),
        revision: 1,
        caret: 2,
        start: 0,
        prefix: "fi".into(),
        trigger: None,
    };
    c.observe(Some(first), now);
    let typed = Context {
        doc: DocumentId(1),
        revision: 2,
        caret: 3,
        start: 0,
        prefix: "fie".into(),
        trigger: None,
    };
    c.observe(Some(typed.clone()), now);
    typed
}

#[test]
fn an_identifier_prefix_is_a_context_and_a_dot_is_a_trigger() {
    let text = "var r Reader\nr.\nread";
    // 「r.」之后：触发字符。
    let dot = context_at(text, 15, 1).expect("点之后有上下文");
    assert_eq!(dot.trigger.as_deref(), Some("."));
    assert_eq!((dot.start, dot.prefix.as_str()), (15, ""));
    // 标识符中间：前缀从词首起。
    let word = context_at(text, 19, 1).expect("词中有上下文");
    assert_eq!(word.trigger, None);
    assert_eq!((word.start, word.prefix.as_str()), (16, "rea"));
    // 空白之后没有上下文。
    assert!(context_at(text, 4, 1).is_none());
    // 有选区时不补全。
    let mut doc = Document::from_text(DocumentId(1), "read");
    doc.selection = winc_core::Selection::new(0, 2);
    assert!(Completion::context(&doc).is_none());
}

#[test]
fn a_request_fires_only_after_the_debounce() {
    let mut c = Completion::default();
    let now = Instant::now();
    let _ = trigger(&mut c, now);
    assert_eq!(c.deadline(), Some(now + DELAY));
    assert!(c.tick(now).is_none());
    let request = c.tick(now + DELAY).expect("到期后有请求");
    assert_eq!((request.caret, request.start), (3, 0));
    assert!(c.tick(now + DELAY).is_none(), "只请求一次");
}

#[test]
fn typing_again_before_the_deadline_moves_the_request_to_the_last_position() {
    let mut c = Completion::default();
    let now = Instant::now();
    let _ = trigger(&mut c, now);
    let typed = Context {
        doc: DocumentId(1),
        revision: 3,
        caret: 4,
        start: 0,
        prefix: "fiel".into(),
        trigger: None,
    };
    c.observe(Some(typed), now + Duration::from_millis(50));
    assert_eq!(c.deadline(), Some(now + Duration::from_millis(50) + DELAY));
    assert!(c.tick(now + DELAY).is_none());
    let request = c.tick(now + Duration::from_millis(50) + DELAY).unwrap();
    assert_eq!(request.caret, 4);
}

#[test]
fn a_stale_response_is_dropped() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    let newer = Context {
        revision: 4,
        caret: 4,
        prefix: "fiel".into(),
        ..ctx
    };
    assert!(!c.response(7, Some(&newer), vec![item("Field")]));
    assert!(!c.shown());
}

#[test]
fn a_response_opens_the_list_and_typing_filters_it() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    assert!(c.response(7, Some(&ctx), vec![item("Field"), item("Foo")]));
    assert!(c.shown());
    assert_eq!(c.rows().len(), 2);
    // 继续输入：前缀变成 fiel，只留下 Field。
    let narrowed = Context {
        revision: 4,
        caret: 4,
        prefix: "fiel".into(),
        ..ctx.clone()
    };
    c.observe(Some(narrowed), now + Duration::from_millis(200));
    assert!(c.shown());
    assert_eq!(c.rows().len(), 1);
    assert_eq!(c.items()[c.rows()[0]].label, "Field");
    // 再输入到 fielf 没有匹配：列表关闭。
    let none = Context {
        revision: 5,
        caret: 5,
        prefix: "fielf".into(),
        ..ctx
    };
    c.observe(Some(none), now + Duration::from_millis(300));
    assert!(!c.shown());
}

#[test]
fn an_older_response_does_not_hide_the_newer_one() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let first = c.tick(now + DELAY).unwrap();
    c.sent(1, first);
    // 第一个请求还没回来，用户又输入了，第二个请求随即发出。
    let newer = Context {
        revision: 4,
        caret: 4,
        prefix: "fiel".into(),
        ..ctx.clone()
    };
    c.sent(2, newer.clone());
    // 旧的响应先到：不是当前请求的，丢弃，但不能吃掉新请求的等待状态。
    assert!(!c.response(1, Some(&ctx), vec![item("Old")]));
    // 新响应到达：应当显示列表。
    assert!(
        c.response(2, Some(&newer), vec![item("Field")]),
        "新请求的响应不应因为旧响应先到而被丢弃"
    );
    assert!(c.shown());
}

#[test]
fn an_empty_response_opens_nothing() {    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    assert!(!c.response(7, Some(&ctx), Vec::new()));
    assert!(!c.shown());
}

#[test]
fn escape_closes_the_list_and_suppresses_the_word_until_a_new_one() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    c.response(7, Some(&ctx), vec![item("Field")]);
    c.escaped();
    assert!(!c.shown());
    // 同一个词继续输入：不再请求。
    let same_word = Context {
        revision: 4,
        caret: 4,
        prefix: "fiel".into(),
        ..ctx.clone()
    };
    c.observe(Some(same_word), now + Duration::from_millis(400));
    assert_eq!(c.deadline(), None);
    // 新的词开始：重新触发。
    let new_word = Context {
        revision: 5,
        caret: 9,
        start: 8,
        prefix: "n".into(),
        trigger: None,
        ..ctx
    };
    c.observe(Some(new_word), now + Duration::from_millis(500));
    assert!(c.deadline().is_some());
}

#[test]
fn moving_the_caret_without_editing_closes_the_list() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    c.response(7, Some(&ctx), vec![item("Field")]);
    let moved = Context {
        caret: 2,
        prefix: "fi".into(),
        ..ctx.clone()
    };
    c.observe(Some(moved), now + Duration::from_millis(200));
    assert!(!c.shown());
}

#[test]
fn leaving_the_identifier_closes_the_list() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    c.response(7, Some(&ctx), vec![item("Field")]);
    c.observe(None, now + Duration::from_millis(200));
    assert!(!c.shown());
}

/// 接受候选：有 `textEdit` 时按它替换；没有时替换已输入的词；整段插入是一个撤销单元。
#[test]
fn accepting_a_candidate_replaces_the_typed_prefix() {
    use winc_lsp::message::{TextEdit, Range, Position};
    let mut doc = Document::from_text(DocumentId(1), "var x Reader
x.Fi
");
    let before = doc.revision;
    let caret = "var x Reader
x.Fi".chars().count();
    doc.selection = winc_core::Selection::point(caret);
    let context = Completion::context(&doc).expect("有上下文");
    assert_eq!(context.prefix, "Fi");
    let item = CompletionItem {
        label: "Field".into(),
        kind: Some(5),
        sort_text: None,
        filter_text: None,
        insert_text: None,
        text_edit: None,
        additional_text_edits: Vec::new(),
    };
    let edit = edit_for(&doc, &context, &item);
    assert_eq!((edit.start, edit.end, edit.text.as_str()), (context.start, caret, "Field"));
    apply_edit(&mut doc, &edit);
    assert_eq!(doc.text().substring(0, doc.text().end_pos()), "var x Reader
x.Field
");
    assert_eq!(doc.selection.head, "var x Reader
x.Field".chars().count());
    // 一次撤销回退整段插入，不残留半个词。
    doc.execute(winc_editor::Command::Undo, 10, 4);
    assert_eq!(doc.text().substring(0, doc.text().end_pos()), "var x Reader
x.Fi
");
    assert!(doc.revision > before);

    // 服务器给了 textEdit：按它的范围替换（这里把 `x.Fi` 整段换成 `x.Reader.Field`）。
    let mut doc = Document::from_text(DocumentId(2), "x.Fi");
    doc.selection = winc_core::Selection::point(4);
    let context = Completion::context(&doc).expect("有上下文");
    let item = CompletionItem {
        text_edit: Some(TextEdit {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 4 },
            },
            new_text: "x.Reader.Field".into(),
        }),
        ..item
    };
    let edit = edit_for(&doc, &context, &item);
    apply_edit(&mut doc, &edit);
    assert_eq!(doc.text().substring(0, doc.text().end_pos()), "x.Reader.Field");
    assert_eq!(doc.selection.head, "x.Reader.Field".chars().count());
}

/// 键盘：列表开着时 ↑↓ 换选中项、Enter/Tab 接受、Esc 关闭；别的键不接管。
#[test]
fn keyboard_navigation_and_acceptance() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    c.response(7, Some(&ctx), vec![item("Field"), item("File")]);
    c.select(1, 8);
    assert_eq!(c.selected(), 1);
    c.select(0, 8);
    assert_eq!(c.selected(), 0);
    // 取走选中项后列表关闭。
    let (chosen, context) = c.take_selected().expect("有选中项");
    assert_eq!(chosen.label, "Field");
    assert_eq!(context.start, 0);
    assert!(!c.shown());
}

/// 接受之后这一段词不会再因为这次插入重新弹出。
#[test]
fn accepting_does_not_reopen_the_list_for_the_inserted_word() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    c.response(7, Some(&ctx), vec![item("Field"), item("Foo")]);
    let (_, context) = c.take_selected().unwrap();
    // 插入后光标在词尾，词的起点不变：上下文与接受时相同。
    let after = Context {
        revision: context.revision + 1,
        caret: context.caret + 5,
        prefix: "Field".into(),
        ..context.clone()
    };
    c.accepted(Some(after.clone()));
    c.observe(Some(after.clone()), now + Duration::from_millis(300));
    assert_eq!(c.deadline(), None, "接受后不再自动弹");
    // 在新的词里输入则照常触发。
    let next_word = Context {
        revision: after.revision + 1,
        caret: after.caret + 1,
        start: after.caret,
        prefix: "x".into(),
        trigger: None,
        ..after
    };
    c.observe(Some(next_word), now + Duration::from_millis(400));
    assert!(c.deadline().is_some());
}

/// 接受函数、方法、构造候选时补上调用括号，光标落在括号之间；字段等不加，已带括号的不重复加。
#[test]
fn function_candidates_are_accepted_with_call_parentheses() {
    let mut doc = Document::from_text(DocumentId(1), "var k KV\nk.g");
    doc.selection = winc_core::Selection::point("var k KV\nk.g".chars().count());
    let context = Completion::context(&doc).expect("有上下文");
    // 函数（kind 3）：加括号，光标在括号之间。
    let edit = edit_for(&doc, &context, &item("get"));
    assert_eq!(edit.text, "get()");
    assert_eq!(edit.caret, Some(4), "光标落在括号之间");
    // 方法（2）与构造（4）同样加括号。
    for kind in [2, 4] {
        let mut method = item("get");
        method.kind = Some(kind);
        assert_eq!(edit_for(&doc, &context, &method).text, "get()");
    }
    // 字段（5）与未知种类不加括号。
    let mut field = item("Key");
    field.kind = Some(5);
    let edit = edit_for(&doc, &context, &field);
    assert_eq!(edit.text, "Key");
    assert_eq!(edit.caret, None);
    // 服务器给的文本已带括号（snippet 风格）时不重复加。
    let mut snipped = item("get");
    snipped.insert_text = Some("get()".into());
    let edit = edit_for(&doc, &context, &snipped);
    assert_eq!(edit.text, "get()");
    assert_eq!(edit.caret, None, "已有括号时光标照旧落在末尾");
    // `textEdit` 来源的函数候选同样补括号，光标在括号之间。
    use winc_lsp::message::{Position, Range, TextEdit};
    let mut edited = item("get");
    edited.text_edit = Some(TextEdit {
        range: Range {
            start: Position { line: 1, character: 2 },
            end: Position { line: 1, character: 3 },
        },
        new_text: "get".into(),
    });
    let edit = edit_for(&doc, &context, &edited);
    assert_eq!(edit.text, "get()");
    assert_eq!(edit.caret, Some(4));
    // 应用后光标确实在括号之间。
    apply_edit(&mut doc, &edit);
    assert_eq!(doc.text().substring(0, doc.text().end_pos()), "var k KV\nk.get()");
    assert_eq!(doc.selection.head, "var k KV\nk.get(".chars().count());
}

/// 接受后把插入的词删回原位：同一位置重新触发（`k.get` 删回 `k.` 再出成员列表）。
#[test]
fn deleting_back_after_accept_retriggers_the_list() {
    let mut c = Completion::default();
    let now = Instant::now();
    // 「k」→「k.」：触发字符请求成员补全。
    let word = Context {
        doc: DocumentId(1),
        revision: 1,
        caret: 1,
        start: 0,
        prefix: "k".into(),
        trigger: None,
    };
    c.observe(Some(word), now);
    let at_dot = Context {
        doc: DocumentId(1),
        revision: 2,
        caret: 2,
        start: 2,
        prefix: String::new(),
        trigger: Some(".".into()),
    };
    c.observe(Some(at_dot.clone()), now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    c.response(7, Some(&at_dot), vec![item("get"), item("set")]);
    // 接受「get」：插入后的上下文。
    let (_, context) = c.take_selected().unwrap();
    let after = Context {
        revision: context.revision + 1,
        caret: context.caret + 3,
        prefix: "get".into(),
        trigger: None,
        ..context.clone()
    };
    c.accepted(Some(after.clone()));
    c.observe(Some(after.clone()), now + Duration::from_millis(300));
    assert_eq!(c.deadline(), None, "插入本身不重新弹出");
    // 把「get」删掉：光标回到点后，词首不变。
    let back = Context {
        revision: after.revision + 1,
        caret: after.start,
        prefix: String::new(),
        trigger: Some(".".into()),
        ..after.clone()
    };
    c.observe(Some(back), now + Duration::from_millis(400));
    assert!(c.deadline().is_some(), "删回词首后同一位置重新触发");
}

/// 候选过多时只保留前面的 `LIMIT` 项。
#[test]
fn only_the_first_items_are_kept() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    let items: Vec<_> = (0..LIMIT + 50).map(|i| item(&format!("C{i:04}"))).collect();
    c.response(7, Some(&ctx), items);
    assert_eq!(c.items().len(), LIMIT);
    assert_eq!(c.rows().len(), LIMIT);
}

/// 滚动：项数多于可见行数时 `scroll_to` 夹到末尾。
#[test]
fn scrolling_clamps_to_the_last_page() {
    let mut c = Completion::default();
    let now = Instant::now();
    let ctx = trigger(&mut c, now);
    let request = c.tick(now + DELAY).unwrap();
    c.sent(7, request);
    let items: Vec<_> = (0..30).map(|i| item(&format!("F{i}"))).collect();
    c.response(7, Some(&ctx), items);
    c.scroll_to(100, 8);
    assert_eq!(c.first(), 22);
    c.scroll_to(0, 8);
    assert_eq!(c.first(), 0);
    // 选中项移出可见范围时随之滚动。
    c.select(20, 8);
    assert_eq!(c.first(), 13);
}
