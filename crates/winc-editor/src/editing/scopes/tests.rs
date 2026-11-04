//! 块识别与折叠范围的测试：缩进分块、Markdown 标题分节、粘滞滚动的固定行与结果复用。

use ropey::Rope;

use super::*;

fn language(file: &str) -> &'static Language {
    winc_syntax::detect(file)
}

/// 不复用上一次的结果、不考虑字符串，返回固定的块（最多 5 层，Tab 宽度 4 列）。
fn pin(
    text: &impl Lines,
    language: &Language,
    rows: &[usize],
    partial: bool,
    mut quoted: impl FnMut(usize) -> bool,
) -> Vec<Scope> {
    sticky(
        text,
        language,
        rows,
        partial,
        5,
        4,
        &mut quoted,
        &mut Vec::new(),
    )
}

fn shown(text: &str, file: &str, first: usize) -> Vec<(usize, Option<usize>)> {
    let rope = Rope::from_str(text);
    let rows: Vec<usize> = (first..first + 6).collect();
    pin(&rope, language(file), &rows, false, |_| false)
        .iter()
        .map(|s| (s.start, s.end))
        .collect()
}

const PYTHON: &str = "\
import os

class A:
    # 注释
    def f(self):
        if x:
            a()

            b()
        return 1

    def g(self):
        pass
";

#[test]
fn blocks_come_from_indentation() {
    // 第一行位于最上方：没有固定的块。
    assert_eq!(shown(PYTHON, "a.py", 0), []);
    // 滚过 class 所在行：固定 class；再向下依次固定 def、if，块中间的空行也属于块。
    assert_eq!(shown(PYTHON, "a.py", 3), [(2, Some(12))]);
    // 滚到 if 所在行：if 行被上方固定的两行遮住，于是固定在第三个位置。
    assert_eq!(
        shown(PYTHON, "a.py", 5),
        [(2, Some(12)), (4, Some(9)), (5, Some(8))]
    );
    // 块末尾的空行不属于块：滚到 g 之前的空行时只剩 class。
    assert_eq!(shown(PYTHON, "a.py", 10), [(2, Some(12))]);
    // 顶格的代码外面没有块。
    assert_eq!(
        depth(
            &Rope::from_str(PYTHON),
            language("a.py"),
            0,
            4,
            &mut |_| { false },
            &[]
        ),
        0
    );
    assert_eq!(
        depth(
            &Rope::from_str(PYTHON),
            language("a.py"),
            6,
            4,
            &mut |_| { false },
            &[]
        ),
        3
    );
}

#[test]
fn a_header_sticks_as_soon_as_it_reaches_the_top() {
    let rope = Rope::from_str(PYTHON);
    let at = |rows: &[usize], partial| -> Vec<usize> {
        pin(&rope, language("a.py"), rows, partial, |_| false)
            .iter()
            .map(|s| s.start)
            .collect()
    };
    // class 所在行滚出半行：class 立即固定在顶部，不必等整行滚出。
    assert_eq!(at(&[2, 3, 4], true), [2]);
    assert_eq!(at(&[2, 3, 4], false), Vec::<usize>::new());
    // 注释滚出半行时，def、if 各自的上半部分也已滚到第二、三个位置之上：都固定。
    assert_eq!(at(&[3, 4, 5], true), [2, 4, 5]);
    assert_eq!(at(&[3, 4, 5], false), [2]);
    // 滚出半行的行不是块的开头：结果不变。
    assert_eq!(at(&[6, 7, 8], true), [2, 4, 5]);

    // Dockerfile 中带续行的命令构成两行的块：到达顶部时即固定，之后被第二行推出
    // 顶部（屏幕上第二行就是块的最后一行）。
    let docker = "FROM go\nRUN a \\\n    b\n\nCOPY . .\n";
    let rope = Rope::from_str(docker);
    let lang = winc_syntax::detect("Dockerfile");
    let scopes = pin(&rope, lang, &[1, 2, 3], true, |_| false);
    assert_eq!(
        scopes,
        [Scope {
            start: 1,
            end: Some(2),
            last_row: Some(1),
        }]
    );
}

/// 复用上一次的结果与每次从头计算的结果相同：在真实文件中依次向下滚动、向上滚动，
/// 以及按固定步长跳转。
#[test]
fn reusing_earlier_results_changes_nothing() {
    let files = [
        (
            "a.rs",
            include_str!("../../../../winc-app/src/view/editor.rs"),
        ),
        ("a.md", include_str!("sections.md")),
        ("a.py", PYTHON),
    ];
    for (file, text) in files {
        // 对照的结果每一步都从文件开头算起，耗时随行数平方增长，因此只取前 300 行。
        let end = text
            .match_indices('\n')
            .nth(300)
            .map_or(text.len(), |(i, _)| i + 1);
        let rope = Rope::from_str(&text[..end]);
        let lang = language(file);
        let lines = rope.len_lines();
        let mut tops: Vec<usize> = (0..lines).collect();
        tops.extend((0..lines).rev());
        tops.extend((0..lines / 4).map(|i| i * 7919 % lines));
        let mut memo = Vec::new();
        for (i, &top) in tops.iter().enumerate() {
            // 每三次中有一次，屏幕行之间夹着折叠隐藏的行。
            let rows: Vec<usize> = if i % 3 == 0 {
                vec![top, top + 1, top + 9, top + 10, top + 11, top + 12]
            } else {
                (top..top + 6).collect()
            };
            let partial = i % 2 == 0;
            let mut quoted = |_| false;
            let reused = sticky(&rope, lang, &rows, partial, 5, 4, &mut quoted, &mut memo);
            let fresh = pin(&rope, lang, &rows, partial, |_| false);
            assert_eq!(reused, fresh, "{file} 第 {top} 行");
        }
    }
}

#[test]
fn a_folded_block_is_pushed_by_the_row_after_it() {
    // 第 2 到 9 行（从 0 起）折叠隐藏：屏幕上 def 的下一行就是 class 之外的 `x = 1`。class、
    // def 两块在屏幕上都只到第 1 个位置（从 0 起），都由第 2 个位置推出顶部，无需找到第 9 行。
    let py = "class A:\n    def f(self):\n        a()\n        b()\n        c()\n        d()\n        e()\n        f()\n        g()\n        h()\nx = 1\n";
    let rope = Rope::from_str(py);
    let found = pin(&rope, language("a.py"), &[0, 1, 10], true, |_| false);
    let rows: Vec<_> = found.iter().map(|s| (s.start, s.last_row)).collect();
    assert_eq!(rows, [(0, Some(1)), (1, Some(1))]);
}

#[test]
fn headers_skip_comments_and_show_the_real_start() {
    // `{` 单独一行时固定其上一行（`void f()`）；顶格的 `#ifdef` 不是块的开头。
    let c = "void f()\n{\n#ifdef X\n    a();\n    b();\n#endif\n}\n";
    assert_eq!(shown(c, "a.c", 3), [(0, Some(4))]);
    // `append(` 下一行是单独的 `{`：两块的开头都是 `append(` 所在行，只固定一次；
    // 收尾的 `)` 不计入块内。
    let py = "def f():\n    out.append(\n        {\n            \"a\": 1,\n            \"b\": 2,\n        }\n    )\n";
    assert_eq!(shown(py, "a.py", 3), [(0, Some(6)), (1, Some(5))]);
    assert_eq!(
        depth(
            &Rope::from_str(py),
            language("a.py"),
            4,
            4,
            &mut |_| false,
            &[]
        ),
        2
    );
    // 方法链跨多行后再接 `{`：开头是 `for` 所在行。
    let chain = "fn f() {\n    for p in xs\n        .iter()\n        .rev()\n    {\n        a();\n        b();\n    }\n}\n";
    assert_eq!(shown(chain, "a.rs", 3), [(0, Some(7)), (1, Some(6))]);
    assert_eq!(shown(chain, "a.rs", 5), [(0, Some(7)), (1, Some(6))]);
    // 已写完的语句之后隔一行的 `{ … }` 是独立的块，上面的语句不算该块的开头。
    let block = "fn f() {\n    let x = 1;\n\n    {\n        a();\n        b();\n    }\n}\n";
    assert_eq!(shown(block, "a.rs", 4), [(0, Some(6)), (3, Some(5))]);
    // `void f()` 下一行是 `{`：`void f()` 一到顶部就固定，滚过 `{` 后仍固定同一行。
    let c = "int x;\nvoid f()\n{\n    a();\n    b();\n    c();\n}\n";
    let rope = Rope::from_str(c);
    for rows in [[1, 2, 3], [2, 3, 4], [3, 4, 5]] {
        let found = pin(&rope, language("a.c"), &rows, true, |_| false);
        assert_eq!(
            found.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
            [(1, Some(5))]
        );
    }
    // 参数列表跨多行：固定 `fn f(` 所在行，从该行到达顶部起一直固定到块体滚完，
    // 滚过 `) -> B {` 所在行时也不变。
    let rust = "fn f(\n    a: A,\n) -> B {\n    body();\n    more();\n}\n";
    assert_eq!(shown(rust, "a.rs", 3), [(0, Some(4))]);
    let rope = Rope::from_str(rust);
    for (rows, partial) in [
        ([0, 1], true),
        ([1, 2], false),
        ([2, 3], false),
        ([2, 3], true),
    ] {
        let found = pin(&rope, language("a.rs"), &rows, partial, |_| false);
        assert_eq!(
            found.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
            [(0, Some(4))]
        );
    }
    assert_eq!(depth(&rope, language("a.rs"), 2, 4, &mut |_| false, &[]), 1);
}

#[test]
fn lines_inside_strings_are_not_code() {
    // 顶格书写的多行字符串既不是块的开头，也不会让外层的块提前结束。
    let py = "def f():\n    q = \"\"\"\nSELECT 1\n\"\"\"\n    a()\n    b()\n";
    let quoted = |l: usize| l == 2 || l == 3;
    let rope = Rope::from_str(py);
    let scopes = pin(&rope, language("a.py"), &[4, 5, 6], false, quoted);
    assert_eq!(
        scopes.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(),
        [(0, Some(5))]
    );
    assert_eq!(
        depth(&rope, language("a.py"), 5, 4, &mut { quoted }, &[]),
        1
    );
    // 不知道哪些行在字符串内时，字符串的最后一行会被误当成块的开头。
    assert_eq!(shown(py, "a.py", 4), [(3, Some(5))]);
}

fn folds(text: &str, file: &str) -> Vec<(usize, usize, usize)> {
    let rope = Rope::from_str(text);
    fold_ranges(&rope, language(file), 4, &mut |_| false)
        .iter()
        .map(|r| (r.start, r.end, r.level))
        .collect()
}

#[test]
fn fold_ranges_follow_indentation() {
    // class 到最后一行有内容的行；方法之间的空行不算在前一个方法里；if 在第三层。
    assert_eq!(
        folds(PYTHON, "a.py"),
        [(2, 12, 1), (4, 9, 2), (5, 8, 3), (11, 12, 2)]
    );
    // `}` 留在外面；顶格的 `#ifdef` 不截断函数。
    let c = "void f() {\n    a();\n#ifdef X\n    b();\n#endif\n}\n";
    assert_eq!(folds(c, "a.c"), [(0, 3, 1)]);
    // 注释不开新块：注释下面缩进更深的行不让注释变成一块。
    let rust = "fn f() {\n    // a\n        x();\n}\n";
    assert_eq!(folds(rust, "a.rs"), [(0, 2, 1)]);
    for line in 0..4 {
        let rope = Rope::from_str(rust);
        let starts = starts_block(&rope, language("a.rs"), line, 4, &mut |_| false);
        assert_eq!(starts, line == 0, "第 {line} 行");
    }
    // 字符串内顶格的行不会结束块。
    let py = "def f():\n    q = \"\"\"\nSELECT\n\"\"\"\n    a()\n";
    let rope = Rope::from_str(py);
    let ranges = fold_ranges(&rope, language("a.py"), 4, &mut |l| l == 2 || l == 3);
    assert_eq!(
        ranges,
        [FoldRange {
            start: 0,
            end: 4,
            level: 1
        }]
    );
}

#[test]
fn markdown_folds_by_heading() {
    let md = "# 一\n\n## 二\n\ntext\n```sh\n# 不是标题\n```\nmore\n\n## 三\nx\n## 空\n";
    assert_eq!(folds(md, "a.md"), [(0, 12, 1), (2, 8, 2), (10, 11, 2)]);
    let rope = Rope::from_str(md);
    assert!(starts_block(&rope, language("a.md"), 2, 4, &mut |_| false));
    assert!(!starts_block(&rope, language("a.md"), 12, 4, &mut |_| {
        false
    }));

    // 逐行询问的结果与整个文件一次计算的结果相同；样本覆盖空的节、代码块内的 `#`，
    // 以及紧跟着下一级标题的标题。
    let samples = [
        md,
        "# a\n# b\n\n\n## c\n### d\n```\n# e\n```\n## f\n\n",
        include_str!("sections.md"),
    ];
    for md in samples {
        let rope = Rope::from_str(md);
        let ranges = heading_ranges(&rope);
        for line in 0..rope.len_lines() {
            assert_eq!(
                starts_block(&rope, language("a.md"), line, 4, &mut |_| false),
                ranges.iter().any(|r| r.start == line),
                "第 {line} 行"
            );
        }
        // 成批询问（中间跳过一些行，如被折叠隐藏的行）与逐行询问的结果相同。
        let lines = (0..rope.len_lines()).filter(|l| l % 3 != 1);
        let single: Vec<usize> = lines
            .clone()
            .filter(|&line| starts_block(&rope, language("a.md"), line, 4, &mut |_| false))
            .collect();
        assert_eq!(
            block_starts(&rope, language("a.md"), lines, 4, &mut |_| false),
            single
        );
    }
}

#[test]
fn markdown_sticks_headings() {
    let md = "# 一\n\n## 二\n\ntext\n```sh\n# 不是标题\n```\nmore\n\n## 三\nx\n";
    assert_eq!(shown(md, "a.md", 3), [(0, Some(11)), (2, Some(8))]);
    // 代码块里的 `#` 不是标题。
    assert_eq!(shown(md, "a.md", 7), [(0, Some(11)), (2, Some(8))]);
    // `## 三` 滚到第二个位置时，替换的是 `## 二`，而不是固定在 `## 二` 下方。
    let rope = Rope::from_str(md);
    let found = pin(&rope, language("a.md"), &[9, 10, 11], true, |_| false);
    assert_eq!(found.iter().map(|s| s.start).collect::<Vec<_>>(), [0, 10]);
    assert_eq!(shown(md, "a.md", 11), [(0, Some(11)), (10, Some(11))]);
}

#[test]
fn ends_far_below_are_not_looked_for() {
    // 块很长、末尾超出向下查找范围时，不返回块的末尾。
    let mut text = String::from("fn f() {\n");
    for _ in 0..500 {
        text.push_str("    x();\n");
    }
    text.push_str("}\n");
    let rope = Rope::from_str(&text);
    let scopes = pin(&rope, language("a.rs"), &[10, 11], false, |_| false);
    assert_eq!(
        scopes,
        [Scope {
            start: 0,
            end: None,
            last_row: None,
        }]
    );
    let scopes = pin(&rope, language("a.rs"), &[480, 481], false, |_| false);
    assert_eq!(scopes[0].end, Some(500));
}
