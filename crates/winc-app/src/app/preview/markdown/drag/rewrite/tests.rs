use super::*;
use winc_preview::markdown::{parse, split_list_items};

/// 按解析结果给出各顶层块的字节范围，与预览拖动时所用的范围相同。
fn ranges(text: &str) -> Vec<Range<usize>> {
    parse(text).with_blank_lines(text).ranges
}

/// 依次应用各处改动。
fn apply(text: &str, moved: &TextMove) -> String {
    let mut out = text.to_owned();
    for edit in &moved.edits {
        out.replace_range(edit.range.clone(), &edit.text);
    }
    out
}

/// 把第 `block` 块移到第 `target` 块之前，返回新原文，并核对各块只换了顺序、没有合并或拆开。
fn move_block(text: &str, block: usize, target: usize) -> String {
    let moved = moved_block(text, &ranges(text), block, target, "\n").expect("应有改动");
    let out = apply(text, &moved);
    let blocks = |text: &str| {
        let mut blocks: Vec<String> = (ranges(text).into_iter())
            .filter(|range| !range.is_empty())
            .map(|range| text[range].trim().to_owned())
            .collect();
        blocks.sort();
        blocks
    };
    assert_eq!(blocks(&out), blocks(text), "块的结构变了：{out:?}");
    out
}

/// 把列表的第 `item` 项移到第 `slot` 项之前，返回改动。
fn move_item(text: &str, item: usize, slot: usize) -> Option<TextMove> {
    let list = split_list_items(text).unwrap();
    moved_list_text(text, 0, &list, item, slot)
}

#[test]
fn moving_a_block_down_keeps_one_blank_line() {
    assert_eq!(move_block("A\n\nB\n\nC\n", 0, 2), "B\n\nA\n\nC\n");
    assert_eq!(move_block("A\n\nB\n\nC\n", 0, 3), "B\n\nC\n\nA\n");
}

#[test]
fn moving_the_last_block_up_leaves_no_blank_line_at_the_end() {
    assert_eq!(move_block("A\n\nB\n\nC\n", 2, 0), "C\n\nA\n\nB\n");
    assert_eq!(move_block("A\n\nB\n\nC\n", 2, 1), "A\n\nC\n\nB\n");
}

#[test]
fn files_without_a_final_newline_stay_without_one() {
    assert_eq!(move_block("A\n\nB", 1, 0), "B\n\nA");
    assert_eq!(move_block("A\n\nB", 0, 2), "B\n\nA");
}

#[test]
fn crlf_files_keep_their_line_endings() {
    let text = "A\r\n\r\nB\r\n\r\nC\r\n";
    let moved = moved_block(text, &ranges(text), 2, 0, "\r\n").unwrap();
    assert_eq!(apply(text, &moved), "C\r\n\r\nA\r\n\r\nB\r\n");
    let moved = moved_block(text, &ranges(text), 0, 3, "\r\n").unwrap();
    assert_eq!(apply(text, &moved), "B\r\n\r\nC\r\n\r\nA\r\n");
}

#[test]
fn headings_keep_a_blank_line_from_the_following_paragraph() {
    // 标题与正文之间没有空行，解析为两块：移动标题时补上空行。
    let text = "# 标题\n正文\n\n第二段\n";
    assert_eq!(move_block(text, 0, 2), "正文\n\n# 标题\n\n第二段\n");
}

#[test]
fn the_gap_left_by_a_moved_block_does_not_merge_its_neighbours() {
    // 列表紧跟在段落之后：移走列表后，段落与原来列表之后的段落之间补上空行。
    let text = "Steps:\n1. a\n2. b\n\nAfter\n\nEnd\n";
    assert_eq!(
        move_block(text, 1, 3),
        "Steps:\n\nAfter\n\n1. a\n2. b\n\nEnd\n"
    );
}

#[test]
fn a_block_moved_up_does_not_join_the_block_before_the_target() {
    // 目标列表紧跟在段落 P 之后：插入的 Q 与 P 之间补上空行。
    assert_eq!(
        move_block("A\n\nP\n- x\n\nQ\n", 3, 2),
        "A\n\nP\n\nQ\n\n- x\n"
    );
    // 目标标题紧跟在列表之后：插入的段落不成为列表最后一项的延续行。
    assert_eq!(
        move_block("- a\n- b\n# H\n\nP\n", 2, 1),
        "- a\n- b\n\nP\n\n# H\n"
    );
}

#[test]
fn extra_blank_lines_move_with_the_separator() {
    // 块间多余的空行解析为空段落，随前一块的分隔一起移走，文首不留空行。
    let text = "A\n\n\n\nB\n\nC\n";
    assert_eq!(move_block(text, 0, 5), "B\n\nC\n\nA\n");
    assert_eq!(move_block(text, 0, 4), "B\n\nA\n\nC\n");
    // B 是 A 之后的下一块，中间只隔着空段落：放在 B 之前不改变顺序。
    assert!(moved_block(text, &ranges(text), 0, 3, "\n").is_none());
}

#[test]
fn indented_code_keeps_its_indentation() {
    // 缩进代码块的范围从缩进之后开始：缩进随块移动，不留在原处。
    let text = "P\n\n    code\n\nQ\n";
    assert_eq!(move_block(text, 1, 3), "P\n\nQ\n\n    code\n");
}

#[test]
fn footnote_definitions_stay_in_place() {
    // 脚注在解析结果中排在最后，原文位置在两段之间。
    let text = "A[^1]\n\n[^1]: 注\n\nB\n";
    assert_eq!(move_block(text, 1, 0), "B\n\nA[^1]\n\n[^1]: 注\n");
}

#[test]
fn a_block_move_edits_only_the_moved_block() {
    let text: String = (0..200).map(|i| format!("第 {i} 段\n\n")).collect();
    let ranges = ranges(&text);
    let moved = moved_block(text.as_str(), &ranges, 0, ranges.len(), "\n").unwrap();
    assert_eq!(moved.edits.len(), 2);
    for edit in &moved.edits {
        assert!(
            edit.range.len() + edit.text.len() <= 16,
            "改动过大：{:?}",
            edit.range
        );
    }
    let out = apply(&text, &moved);
    assert!(out.starts_with("第 1 段\n\n") && out.ends_with("第 199 段\n\n第 0 段\n"));
}

#[test]
fn swapping_identical_blocks_is_not_an_edit() {
    let text = "甲\n\n甲\n";
    assert!(moved_block(text, &ranges(text), 0, 2, "\n").is_none());
    assert!(moved_block(text, &ranges(text), 1, 0, "\n").is_none());
}

#[test]
fn no_op_targets_and_invalid_arguments_return_none() {
    let text = "A\n\nB\n\nC\n";
    let ranges = ranges(text);
    assert!(moved_block(text, &ranges, 0, 0, "\n").is_none());
    assert!(moved_block(text, &ranges, 0, 1, "\n").is_none());
    assert!(moved_block(text, &ranges, 2, 3, "\n").is_none());
    assert!(moved_block(text, &ranges, 3, 0, "\n").is_none());
    assert!(moved_block(text, &ranges, 0, 4, "\n").is_none());
}

#[test]
fn the_caret_follows_its_block() {
    let text = "A\n\nB\n\nC\n";
    let moved = moved_block(text, &ranges(text), 2, 0, "\n").unwrap();
    // 新原文为 "C\n\nA\n\nB\n"：C 末尾的光标仍在 C 末尾，A 开头的光标仍在 A 开头。
    assert_eq!(moved.map(7), 1);
    assert_eq!(moved.map(6), 0);
    assert_eq!(moved.map(0), 3);
    assert_eq!(moved.map(4), 7);

    let moved = moved_block(text, &ranges(text), 0, 3, "\n").unwrap();
    // 新原文为 "B\n\nC\n\nA\n"：A 末尾的光标随 A 移到文末，被移走的空行中的光标落到 B 开头。
    assert_eq!(moved.map(1), 7);
    assert_eq!(moved.map(2), 0);
    assert_eq!(moved.map(3), 0);
    assert_eq!(moved.map(7), 4);
}

#[test]
fn ordered_items_are_renumbered_after_moving() {
    let text = "5. 甲\n6. 乙\n7. 丙\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 3).unwrap()),
        "5. 乙\n6. 丙\n7. 甲\n"
    );
    let text = "1. 甲\n\n2. 乙\n\n3. 丙\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 3).unwrap()),
        "1. 乙\n\n2. 丙\n\n3. 甲\n"
    );
}

#[test]
fn lists_with_equal_numbers_keep_their_numbers() {
    let text = "1. 甲\n1. 乙\n1. 丙\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 3).unwrap()),
        "1. 乙\n1. 丙\n1. 甲\n"
    );
}

#[test]
fn moving_a_list_item_keeps_tight_and_loose_spacing() {
    let text = "- 甲\n- 乙\n- 丙\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 2).unwrap()),
        "- 乙\n- 甲\n- 丙\n"
    );

    let text = "- 甲\n\n- 乙\n\n- 丙\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 2).unwrap()),
        "- 乙\n\n- 甲\n\n- 丙\n"
    );
    // 放到原位或紧邻原位不用改。
    assert!(move_item(text, 0, 0).is_none());
    assert!(move_item(text, 0, 1).is_none());
}

#[test]
fn list_move_keeps_the_blank_line_after_the_list() {
    let text = "- 甲\n- 乙\n\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 2).unwrap()),
        "- 乙\n- 甲\n\n"
    );
    let text = "1. 甲\n2. 乙\n\n";
    assert_eq!(
        apply(text, &move_item(text, 0, 2).unwrap()),
        "1. 乙\n2. 甲\n\n"
    );
}

#[test]
fn a_list_move_edits_only_the_reordered_part() {
    let text = "- a\n- b\n- c\n- d\n";
    let moved = move_item(text, 2, 1).unwrap();
    assert_eq!(apply(text, &moved), "- a\n- c\n- b\n- d\n");
    assert_eq!(moved.edits.len(), 1);
    assert_eq!(moved.edits[0].range, 6..11);
    // 交换两个相同的项不改动原文。
    assert!(move_item("- 甲\n- 甲\n", 0, 2).is_none());
}

#[test]
fn the_caret_at_the_end_of_an_item_stays_with_the_item() {
    for text in ["- a\n- b\n- c\n", "- a\n- b\n- c"] {
        let moved = move_item(text, 0, 3).unwrap();
        let out = apply(text, &moved);
        // 光标在「a」之后，移到末尾后仍在「a」之后。
        assert_eq!(&out[..moved.map(3)], "- b\n- c\n- a", "{text:?}");
        // 「b」开头的光标仍在「b」开头。
        assert_eq!(&out[moved.map(4)..moved.map(4) + 3], "- b", "{text:?}");
    }
}

#[test]
fn the_caret_follows_renumbered_markers() {
    let text = "9. 甲\n   续\n10. 乙\n    续\n";
    let moved = move_item(text, 0, 2).unwrap();
    let out = apply(text, &moved);
    assert_eq!(out, "9. 乙\n   续\n10. 甲\n    续\n");
    // 「甲」之后：编号由 9 变为 10，光标仍在「甲」之后。
    let after_first = "9. 甲".len();
    assert_eq!(&out[..moved.map(after_first)], "9. 乙\n   续\n10. 甲");
    // 续行末尾：续行多缩进一列，光标仍在续行末尾。
    let after_rest = "9. 甲\n   续".len();
    assert_eq!(
        &out[..moved.map(after_rest)],
        "9. 乙\n   续\n10. 甲\n    续"
    );
    // 被移到前面的「乙」：编号由 10 变为 9。
    let after_second = "9. 甲\n   续\n10. 乙".len();
    assert_eq!(&out[..moved.map(after_second)], "9. 乙");
}
