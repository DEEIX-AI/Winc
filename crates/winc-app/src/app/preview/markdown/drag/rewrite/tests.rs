use super::*;
use winc_preview::markdown::split_list_items;

/// 按空行切出各顶层块的字节范围（块的范围到下一块开始为止，含其后的空行）。
fn ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            if let Some(from) = start.take() {
                out.push(from..offset);
            }
        } else if start.is_none() {
            start = Some(offset);
        }
        offset += line.len();
    }
    if let Some(from) = start {
        out.push(from..offset);
    }
    out
}

#[test]
fn ordered_items_are_renumbered_after_moving() {
    let text = "5. 甲\n6. 乙\n7. 丙\n";
    let list = split_list_items(text).unwrap();
    let moved = moved_list_text(text, 0, &list, 0, 3).unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "5. 乙\n6. 丙\n7. 甲\n");
}

#[test]
fn ordered_list_spacing_survives_renumbering() {
    let text = "1. 甲\n\n2. 乙\n\n3. 丙\n";
    let list = split_list_items(text).unwrap();
    let moved = moved_list_text(text, 0, &list, 0, 3).unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "1. 乙\n\n2. 丙\n\n3. 甲\n");
}

#[test]
fn moving_a_list_item_keeps_tight_and_loose_spacing() {
    let text = "- 甲\n- 乙\n- 丙\n";
    let list = split_list_items(text).unwrap();
    let moved = moved_list_text(text, 0, &list, 0, 2).unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "- 乙\n- 甲\n- 丙\n");

    let text = "- 甲\n\n- 乙\n\n- 丙\n";
    let list = split_list_items(text).unwrap();
    let moved = moved_list_text(text, 0, &list, 0, 2).unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "- 乙\n\n- 甲\n\n- 丙\n");

    // 放到原位或紧邻原位不用改。
    assert!(moved_list_text(text, 0, &list, 0, 0).is_none());
    assert!(moved_list_text(text, 0, &list, 0, 1).is_none());
}

#[test]
fn list_move_keeps_the_blank_line_after_the_list() {
    let text = "- 甲\n- 乙\n\n";
    let list = split_list_items(text).unwrap();
    let moved = moved_list_text(text, 0, &list, 0, 2).unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "- 乙\n- 甲\n\n");

    let text = "1. 甲\n2. 乙\n\n";
    let list = split_list_items(text).unwrap();
    let moved = moved_list_text(text, 0, &list, 0, 2).unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "1. 乙\n2. 甲\n\n");
}

#[test]
fn text_move_prefers_chunk_starts_at_boundaries() {
    let moved = TextMove {
        span: 0..2,
        text: "XY".to_owned(),
        chunks: vec![(0..1, 1), (1..2, 0)],
    };
    // 两段共享边界时，起点优先算作后一段的起点。
    assert_eq!(moved.map(0), 1);
    assert_eq!(moved.map(1), 0);
    assert_eq!(moved.map(3), 3);
}

#[test]
fn moving_a_block_down_keeps_one_blank_line() {
    let text = "A\n\nB\n\nC\n";
    let ranges = ranges(text);
    let moved = moved_text(text, &ranges, 0, 2, "\n").unwrap();
    assert_eq!(moved.span, 0..text.find("C").unwrap());
    assert_eq!(moved.text, "B\n\nA\n\n");
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "B\n\nA\n\nC\n");
}

#[test]
fn moving_the_last_block_up_adds_the_separator() {
    let text = "A\n\nB\n\nC\n";
    let ranges = ranges(text);
    // 最后一块只有行尾、没有空行：移到前面时补一个空行。
    let moved = moved_text(text, &ranges, 2, 0, "\n").unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "C\n\nA\n\nB\n\n");
}

#[test]
fn moving_to_the_end_keeps_a_blank_line_before_the_block() {
    let text = "A\n\nB\n\nC\n";
    let ranges = ranges(text);
    let moved = moved_text(text, &ranges, 0, 3, "\n").unwrap();
    assert_eq!(moved.text, "B\n\nC\n\nA\n");
}

#[test]
fn headings_keep_a_blank_line_from_the_following_paragraph() {
    let text = "# 标题\n正文\n\n第二段\n";
    // 标题和正文之间没有空行，是两个块：范围按解析结果给出。
    let title = text.find('\n').unwrap() + 1;
    let body = text.find("\n\n").unwrap() + 1;
    let last = text.find("第二段").unwrap();
    let ranges = vec![0..title, title..body, last..text.len()];
    // 标题和正文之间没有空行：移动标题时补上空行，不把标题和正文合成一段。
    let moved = moved_text(text, &ranges, 0, 2, "\n").unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "正文\n\n# 标题\n\n第二段\n");
}

#[test]
fn no_op_targets_and_invalid_arguments_return_none() {
    let text = "A\n\nB\n\nC\n";
    let ranges = ranges(text);
    assert!(moved_text(text, &ranges, 0, 0, "\n").is_none());
    assert!(moved_text(text, &ranges, 0, 1, "\n").is_none());
    assert!(moved_text(text, &ranges, 2, 3, "\n").is_none());
    assert!(moved_text(text, &ranges, 3, 0, "\n").is_none());
    assert!(moved_text(text, &ranges, 0, 4, "\n").is_none());
}

#[test]
fn files_without_a_final_newline_still_get_a_separator() {
    let text = "A\n\nB";
    let ranges = ranges(text);
    let moved = moved_text(text, &ranges, 1, 0, "\n").unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "B\n\nA\n\n");
}

#[test]
fn crlf_files_keep_their_line_endings() {
    let text = "A\r\n\r\nB\r\n\r\nC\r\n";
    let ranges = ranges(text);
    let moved = moved_text(text, &ranges, 2, 0, "\r\n").unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    assert_eq!(out, "C\r\n\r\nA\r\n\r\nB\r\n\r\n");
}

#[test]
fn blank_lines_before_the_target_stay_with_their_block() {
    let text = "A\n\n\n\nB\n\nC\n";
    let ranges = ranges(text);
    let moved = moved_text(text, &ranges, 0, 2, "\n").unwrap();
    let mut out = text.to_owned();
    out.replace_range(moved.span, &moved.text);
    // A 与 B 之间多余的空行属于分隔，移动时收成一个；C 前面仍保留一个空行。
    assert_eq!(out, "B\n\nA\n\nC\n");
}
