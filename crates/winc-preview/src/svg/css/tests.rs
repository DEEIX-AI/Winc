//! `<style>` 中 CSS 的测试：规则索引与层叠顺序、各种选择器的匹配、非法选择器不 panic。

use super::*;

fn matching<'s>(sheet: &'s Sheet, node: Node) -> Vec<(&'s str, &'s str)> {
    sheet.matching(node).collect()
}

#[test]
fn indexed_rules_keep_cascade_order() {
    // 规则分在 id、类、标签、通用几个桶里，合并后仍按（优先级，先后顺序）排列；元素的
    // `class` 里重复写的类不会让同一条规则出现两次。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><path id="p" class="b a b" data-k="v"/><rect class="a"/></svg>"#;
    let doc = roxmltree::Document::parse(svg).unwrap();
    let path = doc.descendants().find(|n| n.has_tag_name("path")).unwrap();
    let rect = doc.descendants().find(|n| n.has_tag_name("rect")).unwrap();
    let mut sheet = Sheet::default();
    let mut css = String::new();
    for i in 0..200 {
        css.push_str(&format!(".st{i} {{ fill: c{i} }} "));
    }
    css.push_str(
        "#p { k: id } .b { k: b } path { k: tag } .a { k: a } * { k: any } [data-k] { k: attr }
             g .a { k: nested } path.a { k: compound }",
    );
    sheet.add(&css);
    assert_eq!(
        matching(&sheet, path),
        vec![
            ("k", "any"),
            ("k", "tag"),
            ("k", "b"),
            ("k", "a"),
            ("k", "attr"),
            ("k", "compound"),
            ("k", "id"),
        ]
    );
    assert_eq!(matching(&sheet, rect), vec![("k", "any"), ("k", "a")]);
}

#[test]
fn selectors_match_by_specificity_and_structure() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g class="a"><path id="p" class="x y" data-k="v"/></g><path class="x"/></svg>"#;
    let doc = roxmltree::Document::parse(svg).unwrap();
    let paths: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .collect();
    let mut sheet = Sheet::default();
    let unknown = sheet.add(
        "/* c */ path { fill: red } .a > .x { fill: blue } #p { stroke: green }
             .a path.x.y[data-k=v] { opacity: .5 } @media (x) { path { fill: pink } }
             path:first-child { fill: gold } path ~ path { fill: none } svg :hover { x: y }
             path:nth-child(2n) { stroke-width: 3 }",
    );
    assert!(unknown);
    let first = matching(&sheet, paths[0]);
    assert!(first.contains(&("fill", "blue")));
    assert_eq!(first.last(), Some(&("stroke", "green")));
    assert!(first.contains(&("opacity", ".5")));
    let second = matching(&sheet, paths[1]);
    assert_eq!(second, vec![("fill", "red"), ("stroke-width", "3")]);
    assert_eq!(
        (nth("odd"), nth("-n+3"), nth("4")),
        (Some((2, 1)), Some((-1, 3)), Some((0, 4)))
    );
}

#[test]
fn descendant_selectors_do_not_backtrack_forever() {
    // 最近的 `.b` 的父元素不是 `.a`，更上层的 `.b` 的父元素是：需要继续向上查找。
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g class="a"><g class="b"><g class="c"><g class="b"><path/></g></g></g></g></svg>"#;
    let doc = roxmltree::Document::parse(svg).unwrap();
    let path = doc.descendants().find(|n| n.has_tag_name("path")).unwrap();
    let mut sheet = Sheet::default();
    sheet.add(".a > .b path { fill: red } .c > .a path { fill: blue }");
    assert_eq!(matching(&sheet, path), vec![("fill", "red")]);

    // 300 层 `<g>`；选择器含 20 个 `g` 和一个文档中不存在的 `x`。若逐个祖先组合着尝试，
    // 次数会极大，因此必须快速得出不匹配的结论（断言要求 5 秒内）。
    let depth = 300;
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg">{}<path/>{}</svg>"#,
        "<g>".repeat(depth),
        "</g>".repeat(depth)
    );
    let doc = roxmltree::Document::parse(&svg).unwrap();
    let path = doc.descendants().find(|n| n.has_tag_name("path")).unwrap();
    let mut sheet = Sheet::default();
    sheet.add(&format!("x {} path {{ fill: red }}", "g ".repeat(20)));
    let start = std::time::Instant::now();
    assert!(matching(&sheet, path).is_empty());
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn malformed_selectors_do_not_panic() {
    // 非 ASCII 类名若按字节切分会切在字符中间；中文类名应原样保留。
    assert_eq!(
        compound("rect.中文").map(|c| c.classes),
        Some(vec!["中文".into()])
    );
    // 无法识别的非 ASCII 字符按一个字符处理并返回 `None`，不 panic。
    assert!(compound("rect[x]é").is_none());
    // `b` 可以是 `i32::MIN`，用 i32 算 `position - b` 会溢出。
    assert_eq!(nth("-1n-2147483648"), Some((-1, i32::MIN)));
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><path/><path/></svg>"##;
    let doc = roxmltree::Document::parse(svg).unwrap();
    let mut sheet = Sheet::default();
    sheet.add("path:nth-child(-1n-2147483648) { fill: red }");
    assert!(matching(&sheet, doc.descendants().nth(1).unwrap()).is_empty());
}
