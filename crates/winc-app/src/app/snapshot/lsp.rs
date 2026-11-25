//! 补全列表与诊断渲染的测试：列表的翻转、过滤后的滚动，波浪线的错误与警告颜色，
//! 以及鼠标悬停时的诊断气泡。

use winc_core::Selection;
use winit::keyboard::{Key, NamedKey};
use winc_lsp::message::{CompletionItem, Position, Range, Severity, TextEdit};

use super::*;
use crate::app::editor::diagnostics::Problem;

/// 在 `dir` 中创建 Go 文档并打开；返回打开它的界面。
fn go_app(name: &str, text: &str) -> (App, PathBuf) {
    let dir = sample_folder(name);
    let path = dir.join("main.go");
    std::fs::write(&path, text).unwrap();
    let mut app = app_with(&path, false);
    paint(&mut app);
    (app, dir)
}

fn item(label: &str, kind: u32) -> CompletionItem {
    CompletionItem {
        label: label.into(),
        kind: Some(kind),
        sort_text: None,
        filter_text: None,
        insert_text: None,
        text_edit: None,
        additional_text_edits: Vec::new(),
    }
}

/// 让补全会话进入「已显示」状态：会话看到两次版本变化后到期，再把候选交给它。
/// 返回列表对应的上下文（继续输入时据此收窄列表）。
fn show_list(app: &mut App, items: Vec<CompletionItem>) -> crate::app::editor::completion::Context {
    use std::time::Instant;
    let now = Instant::now();
    let doc = app.editor.active().unwrap();
    let mut context = crate::app::editor::completion::Completion::context(doc).unwrap();
    context.revision = doc.revision.saturating_sub(1);
    app.completion.observe(Some(context.clone()), now);
    context.revision = doc.revision;
    app.completion.observe(Some(context.clone()), now);
    app.completion.tick(now + crate::app::editor::completion::DELAY).unwrap();
    app.completion.sent(1, context.clone());
    assert!(app.completion.response(1, Some(&context), items));
    paint(app);
    context
}

/// 列表显示在光标下方；候选按行高排开，键盘换选中项。
#[test]
fn the_completion_card_lists_candidates_below_the_caret() {
    let (mut app, dir) = go_app("lsp-list", "package main\n\nr.\n");
    let text = "package main\n\nr.";
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point(text.chars().count());
    paint(&mut app);
    show_list(
        &mut app,
        vec![item("Reader", 7), item("ReadAll", 3), item("Run", 3)],
    );
    let card = app.hits.completion_card.expect("画出了列表");
    assert_eq!(app.hits.completion.len(), 3);
    assert!(app.hits.completion[0].0.h > 0.0);
    // 卡片在光标下方（光标在窗口中部，下方放得下）。
    assert!(card.y > app.ime.cursor.unwrap().y, "列表应显示在光标下方");
    shoot(&mut app, "lsp-completion");
    // 键盘：↓ 换到第二项，位置在列表中记录为选中。
    assert!(app.completion_key(&Key::Named(NamedKey::ArrowDown)));
    assert_eq!(app.completion.selected(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 继续输入时列表按新的前缀收窄；滚动仍然按收窄后的项数算。
#[test]
fn typing_filters_the_list() {
    let (mut app, dir) = go_app("lsp-filter", "package main

r.
");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point("package main

r.".chars().count());
    paint(&mut app);
    let mut items: Vec<CompletionItem> = (0..20).map(|i| item(&format!("Cand{i:02}"), 3)).collect();
    // 让「Cand1x」与「Cand00」都有，过滤后只剩带 1 的。
    items[0] = item("Cand00", 3);
    let context = show_list(&mut app, items);
    let narrowed = crate::app::editor::completion::Context {
        revision: context.revision + 1,
        caret: context.caret + 1,
        prefix: "Cand1".into(),
        ..context.clone()
    };
    app.completion.observe(Some(narrowed), std::time::Instant::now());
    paint(&mut app);
    assert_eq!(app.completion.rows().len(), 10, "只剩 Cand1x 的 10 项");
    assert!(app.hits.completion_card.is_some());
    shoot(&mut app, "lsp-completion-filtered");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 光标靠近窗口底部、下方放不下时，列表翻到光标上方。
#[test]
fn the_card_flips_above_the_caret_near_the_bottom() {
    let (mut app, dir) = go_app(
        "lsp-flip",
        &format!("package main\n{}r.", "\n".repeat(30)),
    );
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point(doc.text().end_pos());
    paint(&mut app);
    show_list(&mut app, (0..8).map(|i| item(&format!("Cand{i}"), 3)).collect());
    let card = app.hits.completion_card.expect("画出了列表");
    let caret = app.ime.cursor.expect("有光标");
    assert!(card.bottom() <= caret.y, "列表应翻到光标上方：{card:?} {caret:?}");
    shoot(&mut app, "lsp-completion-flipped");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 候选多于可见行数时列表可滚动；选中项移出可见范围时随之滚动。
#[test]
fn a_long_list_scrolls() {
    let (mut app, dir) = go_app("lsp-scroll", "package main\n\nr.\n");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point("package main\n\nr.".chars().count());
    paint(&mut app);
    show_list(&mut app, (0..30).map(|i| item(&format!("Cand{i:02}"), 3)).collect());
    assert_eq!(app.hits.completion.len(), crate::app::lsp::COMPLETION_VISIBLE);
    // 滚轮在列表上滚动列表。
    let card = app.hits.completion_card.unwrap();
    let inside = (card.x + 20.0, card.y + 20.0);
    app.scroll_completion(inside, -200.0);
    assert!(app.completion.first() > 0);
    shoot(&mut app, "lsp-completion-scrolled");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 鼠标点选一行：接受该项并插入。
#[test]
fn clicking_a_row_accepts_it() {
    let (mut app, dir) = go_app("lsp-click", "package main\n\nvar r Reader\nr.\n");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point("package main\n\nvar r Reader\nr.".chars().count());
    paint(&mut app);
    show_list(&mut app, vec![item("ReadAll", 3), item("Reader", 7)]);
    let row = app.hits.completion[1].0;
    let at = (row.x + row.w / 2.0, row.y + row.h / 2.0);
    click(&mut app, at);
    let text = app.editor.active().unwrap().text().substring(0, app.editor.active().unwrap().text().end_pos());
    assert!(text.ends_with("r.Reader\n"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 服务器推送的诊断按 URI 找到打开的文档，存成字符范围；补全响应按请求的上下文对上。
#[test]
fn server_messages_reach_the_open_document_and_the_completion_list() {
    use winc_json::Json;
    use winc_lsp::message::{Diagnostic, Position, Range, Severity};

    let (mut app, dir) = go_app("lsp-messages", "package main

r.
");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point("package main

r.".chars().count());
    let path = app.editor.active().unwrap().path.clone().unwrap();
    let id = app.editor.active().unwrap().id;
    let uri = winc_lsp::message::file_uri(&path).unwrap();
    app.lsp_diagnostics(winc_lsp::message::PublishDiagnosticsParams {
        uri: uri.clone(),
        version: None,
        diagnostics: vec![Diagnostic {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 1 },
            },
            severity: Severity::Error,
            message: "错误".into(),
        }],
    });
    let problems = app.diagnostics.get(id).expect("诊断到了");
    assert_eq!((problems[0].start, problems[0].end), (0, 1));
    paint(&mut app);

    // 补全响应：当前光标处的上下文与请求时一致时打开列表。
    let context = crate::app::editor::completion::Completion::context(app.editor.active().unwrap()).unwrap();
    app.completion.sent(9, context);
    let response = Json::object([(
        "items",
        Json::Array(vec![Json::object([
            ("label", Json::String("Reader".into())),
            ("kind", Json::Number(7.0)),
        ])]),
    )]);
    assert!(app.completion_response(&winc_lsp::rpc::Id::Number(9), &response));
    assert!(app.completion.shown());
    // 对不上的 id 丢弃。
    let context = crate::app::editor::completion::Completion::context(app.editor.active().unwrap()).unwrap();
    app.completion.sent(10, context);
    assert!(!app.completion_response(&winc_lsp::rpc::Id::Number(11), &response));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 波浪线：错误与警告各用自己的颜色画在字符下方；不可见行不画。
#[test]
fn squiggles_use_the_error_and_warning_colors() {
    let (mut app, dir) = go_app(
        "lsp-squiggle",
        "package main\n\nfunc main() {\n\tx := 1\n}\n",
    );
    let doc = app.editor.active().unwrap();
    let line_start = |line: usize| doc.text().line_start(line);
    let id = doc.id;
    let p = app.palette;
    // 第 3 行的 `x` 是错误，第 4 行的 `1` 是警告。
    let error_at = line_start(3);
    let warn_at = line_start(3) + "x := ".chars().count();
    app.diagnostics.replace(
        id,
        vec![
            Problem { start: error_at, end: error_at + 1, severity: Severity::Error, message: "未声明的标识符 x".into() },
            Problem { start: warn_at, end: warn_at + 1, severity: Severity::Warning, message: "多余的赋值".into() },
        ],
    );
    paint(&mut app);
    // 在文字行的底部找到两种颜色的像素。
    let text_rect = app.layout.text;
    let lh = app.fonts.line_height;
    let line_y = text_rect.y + 3.0 * lh;
    let found = |want: winc_render::Color| {
        (0..(text_rect.w as i32)).any(|x| {
            (0..(lh as i32)).any(|y| {
                near(
                    pixel(&app, (text_rect.x + x as f32, line_y + y as f32)),
                    want,
                )
            })
        })
    };
    assert!(found(p.error), "第 3 行应有错误色波浪线");
    assert!(found(p.warning), "第 3 行应有警告色波浪线");
    shoot(&mut app, "lsp-diagnostics");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 鼠标停在诊断范围上并停稳后弹出消息气泡，按严重级别排序；移开后淡出，光标进入不弹。
#[test]
fn hovering_a_diagnostic_shows_its_message_and_leaving_fades_it() {
    use std::time::{Duration, Instant};
    use crate::app::lsp::HOVER_STILL;
    let (mut app, dir) = go_app(
        "lsp-hint",
        "package main\n\nfunc main() {\n\tx := 1\n}\n",
    );
    let doc = app.editor.active().unwrap();
    let line_start = doc.text().line_start(3);
    let id = doc.id;
    app.diagnostics.replace(
        id,
        vec![
            Problem { start: line_start, end: line_start + 1, severity: Severity::Warning, message: "警告的话".into() },
            Problem { start: line_start, end: line_start + 1, severity: Severity::Error, message: "错误的话".into() },
        ],
    );
    let t0 = Instant::now();
    // 输入时光标落在诊断范围内：不弹气泡。
    app.editor.active_mut().unwrap().selection = Selection::point(line_start);
    app.pointer.mouse = (0.0, 0.0);
    app.pointer.cursor_inside = true;
    app.track_diagnostic_hint(t0);
    assert!(app.jumps.name_hint.is_none(), "光标位于诊断处不应自动弹气泡");

    // 鼠标移到诊断所在字符上：还没停稳，先不弹。
    paint(&mut app);
    let caret = app.ime.cursor.expect("光标在屏幕上");
    app.pointer.mouse = (caret.x + 2.0, caret.y + 2.0);
    app.track_diagnostic_hint(t0);
    assert!(app.jumps.name_hint.is_none(), "刚移到诊断上未停稳不弹");

    // 停满悬停延迟：弹出气泡。
    app.track_diagnostic_hint(t0 + HOVER_STILL);
    paint(&mut app);
    shoot(&mut app, "lsp-diagnostic-hint");
    let hint = app.jumps.name_hint.as_ref().expect("弹出了气泡");
    assert!(hint.diagnostic);
    // 气泡锚定在诊断起点所在的文字处，而不是跟着鼠标。
    assert_eq!(hint.anchor, line_start, "气泡锚定诊断起点");
    // 多条按严重级别从高到低。
    assert_eq!(hint.text, "错误的话\n警告的话");
    // 鼠标停在原处：保持显示，不因超时消失。
    app.jumps.name_hint.as_mut().unwrap().text = String::new();
    app.track_diagnostic_hint(t0 + HOVER_STILL + Duration::from_millis(100));
    assert!(app.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic), "停着不动保持显示");

    // 鼠标移出诊断范围：气泡淡出。
    app.pointer.mouse = (0.0, 0.0);
    app.track_diagnostic_hint(t0 + HOVER_STILL + Duration::from_millis(200));
    assert!(app.jumps.name_hint.is_none(), "移开后气泡淡出");
    // 移回来：未停稳先不弹，停稳后重新弹出。
    app.pointer.mouse = (caret.x + 2.0, caret.y + 2.0);
    app.track_diagnostic_hint(t0 + HOVER_STILL + Duration::from_millis(300));
    assert!(app.jumps.name_hint.is_none(), "移回后未停稳不弹");
    app.track_diagnostic_hint(t0 + HOVER_STILL + Duration::from_millis(300) + HOVER_STILL);
    assert!(app.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic), "停稳后重新弹出");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 打字时服务器对每个中间状态推送新诊断：按键后待发作废，静止的鼠标不重弹；
/// 鼠标再次移动到诊断上停稳才弹（VSCode 的悬停语义：悬停由移动触发）。
#[test]
fn typing_through_intermediate_diagnostics_does_not_pop_the_hint() {
    use std::time::{Duration, Instant};
    use crate::app::lsp::HOVER_STILL;
    let (mut app, dir) = go_app("lsp-hint-typing", "package main\n\nfunc main() {\n\tk.\n}\n");
    let doc = app.editor.active().unwrap();
    let line_start = doc.text().line_start(3);
    let id = doc.id;
    app.editor.active_mut().unwrap().selection = Selection::point(line_start);
    app.diagnostics.replace(
        id,
        vec![
            Problem { start: line_start, end: line_start + 2, severity: Severity::Error, message: "expected selector".into() },
        ],
    );
    paint(&mut app);
    let caret = app.ime.cursor.expect("光标在屏幕上");
    let t0 = Instant::now();
    // 鼠标移到诊断上并停稳：弹出气泡。
    app.pointer.cursor_inside = true;
    app.pointer.mouse = (caret.x + 2.0, caret.y + 2.0);
    app.track_diagnostic_hint(t0);
    app.track_diagnostic_hint(t0 + HOVER_STILL);
    assert!(
        app.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic),
        "停稳后弹出气泡"
    );

    // 模拟一次按键：气泡淡出，悬停待发作废。
    let typed = t0 + HOVER_STILL + Duration::from_millis(50);
    app.dismiss_hint();
    app.lsp.disarm_diagnostic_hint();
    // 服务器随后推送中间态的新诊断（消息变了）。
    app.diagnostics.replace(
        id,
        vec![
            Problem { start: line_start, end: line_start + 4, severity: Severity::Error, message: "k.tt undefined".into() },
        ],
    );
    // 静止的鼠标不重弹：不管过了多久。
    app.track_diagnostic_hint(typed + Duration::from_millis(100));
    assert!(app.jumps.name_hint.is_none(), "按键后不因新诊断重弹");
    app.track_diagnostic_hint(typed + Duration::from_millis(200));
    assert!(app.jumps.name_hint.is_none(), "持续不弹");
    // 停手很久之后：鼠标从没动过，依然不弹。
    let later = typed + Duration::from_secs(2);
    app.track_diagnostic_hint(later);
    assert!(
        app.jumps.name_hint.is_none(),
        "停手后静止的鼠标不重弹，要等鼠标移动"
    );

    // 鼠标移动（仍在诊断范围内）：重新待发，停稳后弹出最新消息。
    app.pointer.mouse = (caret.x + 3.0, caret.y + 2.0);
    app.track_diagnostic_hint(later + Duration::from_millis(10));
    assert!(app.jumps.name_hint.is_none(), "刚移动还没停稳");
    app.track_diagnostic_hint(later + Duration::from_millis(10) + HOVER_STILL);
    assert_eq!(
        app.jumps.name_hint.as_ref().expect("移动停稳后弹出").text,
        "k.tt undefined"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 补全列表显示期间鼠标移到诊断上：不弹诊断气泡（建议列表打开时隐藏悬停，同 `VSCode`）。
#[test]
fn the_diagnostic_hint_stays_hidden_while_the_completion_card_is_open() {
    use std::time::Instant;
    use crate::app::lsp::HOVER_STILL;
    let (mut app, dir) = go_app("lsp-hint-completion", "package main\n\nfunc main() {\n\tk.\n}\n");
    let caret_at = "package main\n\nfunc main() {\n\tk.".chars().count();
    app.editor.active_mut().unwrap().selection = Selection::point(caret_at);
    let doc = app.editor.active().unwrap();
    let line_start = doc.text().line_start(3);
    let id = doc.id;
    app.diagnostics.replace(
        id,
        vec![
            Problem { start: line_start, end: line_start + 2, severity: Severity::Error, message: "expected selector".into() },
        ],
    );
    paint(&mut app);
    show_list(&mut app, vec![item("tt", 5)]);
    assert!(app.completion.shown(), "补全列表开着");
    let caret = app.ime.cursor.expect("光标在屏幕上");
    let t0 = Instant::now();
    app.pointer.cursor_inside = true;
    app.pointer.mouse = (caret.x + 2.0, caret.y + 2.0);
    app.track_diagnostic_hint(t0);
    app.track_diagnostic_hint(t0 + HOVER_STILL);
    assert!(
        app.jumps.name_hint.is_none(),
        "补全列表打开时不弹诊断气泡"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 每行行首的种类图标按语法色着色：函数用函数色、字段用属性色（同大纲的 `symbol_look`）。
#[test]
fn completion_rows_show_colored_kind_icons() {
    let (mut app, dir) = go_app("lsp-kind", "package main\n\nr.\n");
    let doc = app.editor.active_mut().unwrap();
    doc.selection = Selection::point("package main\n\nr.".chars().count());
    paint(&mut app);
    show_list(&mut app, vec![item("ReadAll", 3), item("Reader", 5)]);
    let card = app.hits.completion_card.expect("画出了列表");
    let p = app.palette;
    let found = |want: winc_render::Color| {
        (0..card.w as i32).any(|x| {
            (0..card.h as i32)
                .any(|y| near(pixel(&app, (card.x + x as f32, card.y + y as f32)), want))
        })
    };
    assert!(found(p.syntax_function), "函数候选用函数色图标");
    assert!(found(p.syntax_property), "字段候选用属性色图标");
    shoot(&mut app, "lsp-completion-kind");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 悬停到期时由定时唤醒就地评估：鼠标停下后没有新事件，不必再等鼠标移动才弹。
#[test]
fn the_hover_deadline_wakes_the_event_loop() {
    use std::time::Instant;
    use crate::app::lsp::HOVER_STILL;
    let (mut app, dir) = go_app("lsp-hover-wake", "package main\n\nfunc main() {\n\tx := 1\n}\n");
    let doc = app.editor.active().unwrap();
    let line_start = doc.text().line_start(3);
    let id = doc.id;
    app.editor.active_mut().unwrap().selection = Selection::point(line_start);
    app.diagnostics.replace(
        id,
        vec![Problem {
            start: line_start,
            end: line_start + 1,
            severity: Severity::Error,
            message: "这里有错".into(),
        }],
    );
    paint(&mut app);
    let caret = app.ime.cursor.expect("光标在屏幕上");
    app.pointer.cursor_inside = true;
    app.pointer.mouse = (caret.x + 2.0, caret.y + 2.0);
    let t0 = Instant::now();
    app.track_diagnostic_hint(t0);
    assert!(app.jumps.name_hint.is_none(), "刚移入还没停稳");
    // 待发暴露到期时刻，事件循环据此安排唤醒。
    let due = app.lsp.hover_deadline().expect("悬停待发中");
    assert_eq!(due, t0 + HOVER_STILL);
    assert_eq!(app.hover_tick(t0), Some(due), "到期前返回唤醒时刻");
    // 到点：直接评估并弹出，不再依赖下一次鼠标事件。
    assert_eq!(app.hover_tick(due), None);
    assert!(
        app.jumps.name_hint.as_ref().is_some_and(|h| h.diagnostic),
        "到期即弹出"
    );
    // 评估后不再有待发，事件循环不会再被反复唤醒。
    assert_eq!(app.lsp.hover_deadline(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 气泡左侧的严重级别图标：错误用错误色、警告用警告色；长消息折成多行、卡片随之变高。
#[test]
fn the_diagnostic_bubble_shows_level_icons_and_wraps_long_messages() {
    use std::time::Instant;
    use crate::app::lsp::HOVER_STILL;
    let (mut app, dir) = go_app(
        "lsp-hint-level",
        "package main\n\nfunc main() {\n\tx := 1\n}\n",
    );
    let doc = app.editor.active().unwrap();
    let line_start = doc.text().line_start(3);
    let id = doc.id;
    app.editor.active_mut().unwrap().selection = Selection::point(line_start);
    let long = "这一条消息非常长，应当在气泡里折成好几行显示，而不是把卡片撑到编辑区外面去，\
                读起来也更舒服一些，这句话再长一点，确保一定超出一行的宽度看到折行效果。";
    app.diagnostics.replace(
        id,
        vec![Problem {
            start: line_start,
            end: line_start + 1,
            severity: Severity::Warning,
            message: long.into(),
        }],
    );
    paint(&mut app);
    let caret = app.ime.cursor.expect("光标在屏幕上");
    app.pointer.cursor_inside = true;
    app.pointer.mouse = (caret.x + 2.0, caret.y + 2.0);
    let t0 = Instant::now();
    app.track_diagnostic_hint(t0);
    app.track_diagnostic_hint(t0 + HOVER_STILL);
    let hint = app.jumps.name_hint.as_ref().expect("弹出了气泡");
    assert_eq!(hint.level, Some(1), "警告级别传给了气泡");
    paint(&mut app);
    let p = app.palette;
    let editor = app.layout.editor;
    let found = |want: winc_render::Color| {
        (0..(editor.h as i32)).step_by(2).any(|y| {
            (0..(editor.w as i32))
                .step_by(2)
                .any(|x| near(pixel(&app, (editor.x + x as f32, editor.y + y as f32)), want))
        })
    };
    assert!(found(p.warning), "气泡里有警告色图标");
    shoot(&mut app, "lsp-hint-level");
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn a_text_edit_from_the_server_is_applied() {
    let (mut app, dir) = go_app("lsp-textedit", "package main\n\nvar r Reader\nr.Re\n");
    let doc = app.editor.active_mut().unwrap();
    let end = "package main\n\nvar r Reader\nr.Re".chars().count();
    doc.selection = Selection::point(end);
    paint(&mut app);
    let mut item = item("ReadAll", 3);
    item.text_edit = Some(TextEdit {
        range: Range {
            start: Position { line: 3, character: 2 },
            end: Position { line: 3, character: 4 },
        },
        new_text: "ReadAll".into(),
    });
    show_list(&mut app, vec![item]);
    app.accept_completion();
    let text = app.editor.active().unwrap().text().substring(0, app.editor.active().unwrap().text().end_pos());
    // 函数候选（kind 3）接受时补上调用括号。
    assert!(text.ends_with("r.ReadAll()\n"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
