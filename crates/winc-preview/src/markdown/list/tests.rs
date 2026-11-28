use super::split_list_items;

#[test]
fn splits_only_at_sibling_markers() {
    let text = "- 甲\n  - 甲一\n  - 甲二\n- 乙\n";
    let chunks: Vec<&str> = split_list_items(text)
        .unwrap()
        .ranges
        .iter()
        .map(|r| &text[r.clone()])
        .collect();
    assert_eq!(chunks, ["- 甲\n  - 甲一\n  - 甲二\n", "- 乙\n"]);

    let text = "- [ ] 甲\n- [x] 乙\n";
    assert_eq!(split_list_items(text).unwrap().ranges.len(), 2);

    // 围栏代码块里看起来像列表项的行不切分。
    let text = "- 甲\n  ```\n  - 不是项\n  ```\n- 乙\n";
    let chunks: Vec<&str> = split_list_items(text)
        .unwrap()
        .ranges
        .iter()
        .map(|r| &text[r.clone()])
        .collect();
    assert_eq!(chunks, ["- 甲\n  ```\n  - 不是项\n  ```\n", "- 乙\n"]);

    assert!(split_list_items("1. 甲\n").is_none());
    assert!(split_list_items("- 甲\n").is_none());
}

#[test]
fn ordered_lists_renumber_across_marker_widths() {
    let list = split_list_items("5. 甲\n6. 乙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(ordered.start(), 5);
    assert_eq!(ordered.renumber("5. 甲", 0).unwrap(), "5. 甲");

    // 第 10 个槽位从一位变两位，续行也要右移一格。
    let last = ordered.renumber("1. 第项\n   续\n", 5).unwrap();
    assert_eq!(last, "10. 第项\n    续\n");

    // 从两位变回一位时去掉续行多出来的缩进。
    let list = split_list_items("1. 甲\n10. 乙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(
        ordered.renumber("10. 乙\n    续\n", 0).unwrap(),
        "1. 乙\n   续\n"
    );

    // 分隔符保持 `)`，起始编号保持 10。
    let list = split_list_items("10) 甲\n11) 乙\n12) 丙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(
        ordered.renumber("10) 甲\n    续\n", 0).unwrap(),
        "10) 甲\n    续\n"
    );
    assert_eq!(
        ordered.renumber("12) 丙\n    续\n", 0).unwrap(),
        "10) 丙\n    续\n"
    );
}

#[test]
fn lists_with_equal_numbers_keep_their_markers() {
    let list = split_list_items("1. 甲\n1. 乙\n1. 丙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(
        ordered.renumber("1. 丙\n   续\n", 0).unwrap(),
        "1. 丙\n   续\n"
    );
    assert_eq!(ordered.renumber("1. 甲", 2).unwrap(), "1. 甲");

    // 只有部分编号相同时仍按位置重编号。
    let list = split_list_items("1. 甲\n1. 乙\n2. 丙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(ordered.renumber("1. 甲", 2).unwrap(), "3. 甲");
}

#[test]
fn a_tab_after_the_marker_keeps_the_content_column() {
    // `9.` 与 `10.` 之后的制表符均把内容推到第 4 列：续行的缩进不变。
    let list = split_list_items("9.\t甲\n10.\t乙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(
        ordered.renumber("9.\t甲\n    续\n", 1).unwrap(),
        "10.\t甲\n    续\n"
    );
    assert_eq!(
        ordered.renumber("10.\t乙\n    续\n", 0).unwrap(),
        "9.\t乙\n    续\n"
    );

    // `99.` 之后的制表符推到第 4 列，`100.` 之后推到第 8 列：续行右移 4 列。
    let list = split_list_items("99.\t甲\n100.\t乙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(
        ordered.renumber("99.\t甲\n    续\n", 1).unwrap(),
        "100.\t甲\n        续\n"
    );
}

#[test]
fn wide_spacing_after_the_marker_counts_as_one_column() {
    // 标记后空白达到 5 列时内容从标记后 1 列开始（其余空白属于缩进代码块），续行按 1 列计。
    let list = split_list_items("9.      代码\n10. 乙\n").unwrap();
    let ordered = list.ordered.as_ref().unwrap();
    assert_eq!(
        ordered.renumber("9.      代码\n   续\n", 1).unwrap(),
        "10.      代码\n    续\n"
    );
}
