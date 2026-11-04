//! `winc-i18n` 的测试：语言包与目录的一致性、各语言的参数相同、消息的格式化，以及 `t!` 调用与
//! 语言包的对应。
//!
//! 语言包的检查直接调用 crate 内的私有函数，因此测试写在 crate 内部。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fluent_syntax::ast;

use super::*;

/// 一个语言包中每条消息的 id 和它引用的参数名。
type Messages = BTreeMap<String, BTreeSet<String>>;

/// 收集 `pattern` 中引用的参数名。
fn pattern_vars(pattern: &ast::Pattern<&str>, vars: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            expression_vars(expression, vars);
        }
    }
}

/// 收集 `expression` 中引用的参数名，包括选择表达式各分支中的引用。
fn expression_vars(expression: &ast::Expression<&str>, vars: &mut BTreeSet<String>) {
    match expression {
        ast::Expression::Select { selector, variants } => {
            inline_vars(selector, vars);
            for variant in variants {
                pattern_vars(&variant.value, vars);
            }
        }
        ast::Expression::Inline(inline) => inline_vars(inline, vars),
    }
}

/// 收集行内表达式 `inline` 中引用的参数名。
fn inline_vars(inline: &ast::InlineExpression<&str>, vars: &mut BTreeSet<String>) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            vars.insert(id.name.to_owned());
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            for argument in &arguments.positional {
                inline_vars(argument, vars);
            }
            for argument in &arguments.named {
                inline_vars(&argument.value, vars);
            }
        }
        ast::InlineExpression::Placeable { expression } => expression_vars(expression, vars),
        _ => {}
    }
}

/// 收集 `pattern` 中引用的其他消息（`{ common-trash }` 这样的消息引用）的 id。
fn pattern_references(pattern: &ast::Pattern<&str>, ids: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            expression_references(expression, ids);
        }
    }
}

/// 收集 `expression` 中引用的其他消息的 id，包括选择表达式各分支中的引用。
fn expression_references(expression: &ast::Expression<&str>, ids: &mut BTreeSet<String>) {
    let inline = |inline: &ast::InlineExpression<&str>, ids: &mut BTreeSet<String>| match inline {
        ast::InlineExpression::MessageReference { id, .. } => {
            ids.insert(id.name.to_owned());
        }
        ast::InlineExpression::Placeable { expression } => expression_references(expression, ids),
        _ => {}
    };
    match expression {
        ast::Expression::Select { selector, variants } => {
            inline(selector, ids);
            for variant in variants {
                pattern_references(&variant.value, ids);
            }
        }
        ast::Expression::Inline(expression) => inline(expression, ids),
    }
}

/// 源语言的语言包中被其他消息引用的消息。
fn referenced_messages() -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for (_, source) in catalog::sources(Locale::SOURCE) {
        let resource =
            fluent_syntax::parser::parse(source).unwrap_or_else(|(_, e)| panic!("{e:?}"));
        for entry in resource.body {
            if let ast::Entry::Message(message) = entry
                && let Some(value) = &message.value
            {
                pattern_references(value, &mut ids);
            }
        }
    }
    ids
}

/// 解析 `locale` 的语言包，返回每个文件中的消息。
fn messages(locale: Locale) -> BTreeMap<&'static str, Messages> {
    catalog::sources(locale)
        .map(|(file, source)| {
            let resource = fluent_syntax::parser::parse(source)
                .unwrap_or_else(|(_, errors)| panic!("{}/{file}.ftl：{errors:?}", locale.tag()));
            // 每条消息记录默认值中引用的参数名，选择表达式各分支中的引用也算在内。
            let mut messages = Messages::new();
            for entry in resource.body {
                if let ast::Entry::Message(message) = entry {
                    let mut vars = BTreeSet::new();
                    if let Some(value) = &message.value {
                        pattern_vars(value, &mut vars);
                    }
                    messages.insert(message.id.name.to_owned(), vars);
                }
            }
            (file, messages)
        })
        .collect()
}

/// 返回 `locales/` 目录。
fn locales_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("locales")
}

#[test]
fn each_locale_has_exactly_the_catalog_files() {
    let expected: BTreeSet<String> = catalog::FILES.iter().map(|f| format!("{f}.ftl")).collect();
    for locale in Locale::ALL {
        let found: BTreeSet<String> = std::fs::read_dir(locales_dir().join(locale.tag()))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, expected, "{}", locale.tag());
    }
    let dirs: BTreeSet<String> = std::fs::read_dir(locales_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let tags: BTreeSet<String> = Locale::ALL.iter().map(|l| l.tag().to_owned()).collect();
    assert_eq!(dirs, tags);
}

#[test]
fn every_locale_parses_into_a_bundle() {
    for locale in Locale::ALL {
        bundle(locale);
    }
}

#[test]
fn locales_define_the_same_messages_with_the_same_arguments() {
    let source = messages(Locale::SOURCE);
    for locale in Locale::ALL {
        let other = messages(locale);
        for file in catalog::FILES {
            let (a, b) = (&source[file], &other[file]);
            let missing: Vec<_> = a.keys().filter(|id| !b.contains_key(*id)).collect();
            let extra: Vec<_> = b.keys().filter(|id| !a.contains_key(*id)).collect();
            assert!(
                missing.is_empty(),
                "{}/{file}.ftl 缺少：{missing:?}",
                locale.tag()
            );
            assert!(
                extra.is_empty(),
                "{}/{file}.ftl 多出：{extra:?}",
                locale.tag()
            );
            for (id, vars) in a {
                assert_eq!(vars, &b[id], "{}/{file}.ftl：{id} 的参数", locale.tag());
            }
        }
    }
}

#[test]
fn every_message_formats_without_errors() {
    for locale in Locale::ALL {
        let bundle = bundle(locale);
        for messages in messages(locale).values() {
            for (id, vars) in messages {
                // 每个参数分别以单数（1）和复数（3）代入，覆盖选择表达式的各分支；
                // `$platform` 和查找时一样总是带上，三种系统都试一遍。
                for (n, platform) in [1, 2]
                    .into_iter()
                    .flat_map(|n| ["macos", "windows", "linux"].map(|platform| (n, platform)))
                {
                    let mut args = FluentArgs::new();
                    for var in vars {
                        args.set(var.as_str(), n);
                    }
                    args.set("platform", platform);
                    let pattern = bundle.get_message(id).and_then(|m| m.value());
                    let pattern =
                        pattern.unwrap_or_else(|| panic!("{}：{id} 没有值", locale.tag()));
                    let mut errors = Vec::new();
                    let text = bundle.format_pattern(pattern, Some(&args), &mut errors);
                    assert!(errors.is_empty(), "{}：{id}：{errors:?}", locale.tag());
                    assert!(!text.trim().is_empty(), "{}：{id} 为空", locale.tag());
                }
            }
        }
    }
}

/// 源代码中的一次 `t!` 调用：消息 id、传入的参数名和所在位置。
struct Call {
    id: String,
    args: BTreeSet<String>,
    at: String,
}

/// 收集 `dir` 下各 `.rs` 文件中的 `t!` 调用。
fn calls_in(dir: &Path, calls: &mut Vec<Call>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            calls_in(&path, calls);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            for (offset, _) in source.match_indices("t!(") {
                // 排除名称以 `t` 结尾的其他宏（如 `print!`、`assert!`）和字符串中的 `t!(`。
                let before = source[..offset].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '"') {
                    continue;
                }
                // 排除注释和文档中的示例。
                let line_start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
                if source[line_start..offset].trim_start().starts_with("//") {
                    continue;
                }
                let line = source[..offset].matches('\n').count() + 1;
                let at = format!("{}:{line}", path.display());
                // 宏的参数可以换行书写，id 之前允许空白。
                let Some(rest) = source[offset + 3..].trim_start().strip_prefix('"') else {
                    continue;
                };
                calls.push(parse_call(rest, at));
            }
        }
    }
}

/// 解析 `t!(` 和 id 的左引号之后的文本 `rest`：消息 id，以及最外层逗号之后的 `名称 =` 参数。
fn parse_call(rest: &str, at: String) -> Call {
    let end = rest.find('"').unwrap();
    let id = rest[..end].to_owned();
    let mut args = BTreeSet::new();
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    let mut segment_start = None;
    for (i, c) in rest[end + 1..].char_indices() {
        let i = end + 1 + i;
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => break,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => segment_start = Some(i + 1),
            '=' if depth == 0 => {
                if let Some(start) = segment_start.take() {
                    args.insert(rest[start..i].trim().to_owned());
                }
            }
            _ => {}
        }
    }
    Call { id, args, at }
}

#[test]
fn source_calls_match_the_catalog() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut calls = Vec::new();
    for entry in std::fs::read_dir(crates).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            calls_in(&src, &mut calls);
        }
    }
    let catalog: Messages = messages(Locale::SOURCE).into_values().flatten().collect();
    let mut used = BTreeSet::new();
    for call in &calls {
        let mut vars = catalog
            .get(&call.id)
            .unwrap_or_else(|| panic!("{}：语言包中没有消息 {}", call.at, call.id))
            .clone();
        // `$platform` 由查找时自动带上（`PLATFORM`），调用处不传。
        vars.remove("platform");
        assert_eq!(call.args, vars, "{}：{} 的参数", call.at, call.id);
        used.insert(call.id.as_str());
    }
    // 只在其他消息中引用的（如各系统叫法不同的 `common-trash`）也算用到。
    let referenced = referenced_messages();
    let unused: Vec<_> = catalog
        .keys()
        .filter(|id| !used.contains(id.as_str()) && !referenced.contains(*id))
        .collect();
    assert!(unused.is_empty(), "语言包中有未使用的消息：{unused:?}");
}

#[test]
fn numbers_select_the_plural_form_of_each_locale() {
    let id = "test-files";
    let source =
        "test-files = { $count -> \n [one] { $count } file\n *[other] { $count } files\n}\n";
    let mut bundle = Bundle::new_concurrent(vec![Locale::EnUs.langid()]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(FluentResource::try_new(source.to_owned()).unwrap())
        .unwrap();
    let pattern = bundle.get_message(id).unwrap().value().unwrap();
    for (n, expected) in [(1, "1 file"), (3, "3 files")] {
        let mut args = FluentArgs::new();
        args.set("count", n);
        let mut errors = Vec::new();
        assert_eq!(
            bundle.format_pattern(pattern, Some(&args), &mut errors),
            expected
        );
    }
}

#[test]
fn tags_match_the_nearest_supported_locale() {
    assert_eq!(Locale::matching("zh-Hans-CN"), Some(Locale::ZhCn));
    assert_eq!(Locale::matching("zh_TW"), Some(Locale::ZhCn));
    assert_eq!(Locale::matching("en-GB"), Some(Locale::EnUs));
    assert_eq!(Locale::matching("en_US@rg=cnzzzz"), Some(Locale::EnUs));
    assert_eq!(Locale::matching("fr-FR"), None);
    assert_eq!(Locale::matching(""), None);
}

#[test]
fn choices_round_trip_through_their_keys() {
    for choice in LocaleChoice::ALL {
        assert_eq!(LocaleChoice::from_key(choice.key()), Some(choice));
    }
    assert_eq!(LocaleChoice::from_key("zh"), None);
}

#[test]
fn switching_the_locale_changes_cached_text() {
    // 测试并行运行，此处只使用局部的槽位和显式的语言，不改动全局的当前语言。
    let slot = Slot::new();
    let zh = slot.0[Locale::ZhCn.index()]
        .get_or_init(|| lookup(Locale::ZhCn, "common-cancel", None).leak());
    let en = slot.0[Locale::EnUs.index()]
        .get_or_init(|| lookup(Locale::EnUs, "common-cancel", None).leak());
    // 两个槽位互不影响：同一个 `id` 在不同语言下分别保存首次查找的结果。
    assert_eq!((*zh, *en), ("取消", "Cancel"));
}
