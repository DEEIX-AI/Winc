//! `diff_view` 模块的测试：两侧各自排出的显示行、滚动位置与显示行的换算、长段折叠与展开、
//! 逐词标记、重新比较以后沿用状态。

use super::*;

fn view(old: &str, new: &str) -> DiffView {
    let language = winc_syntax::detect("a.rs");
    DiffView::new(
        DiffSide::new("old", old, None, language),
        DiffSide::new("new", new, None, language),
    )
}

fn numbered(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

fn line(line: usize, hunk: Option<usize>) -> SideRow {
    SideRow::Line { line, hunk }
}

#[test]
fn a_long_deletion_in_the_middle_is_scrolled_through_line_by_line() {
    // HEAD 第 30 到 80 行在工作区里删掉了：滚动位置给这段留 50 行，滚过去时左侧一行行走，
    // 右侧停在删除的地方不动。
    let old = numbered(100);
    let new: String = old
        .lines()
        .enumerate()
        .filter(|(i, _)| !(30..80).contains(i))
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    let mut v = view(&old, &new);
    v.set_collapse(false, 0);
    assert_eq!(v.row_count(), 50);
    assert_eq!(v.scroll_rows(), 100);
    assert_eq!(v.hunk_span(0), 30.0..80.0);

    // 一屏 20 行。
    let at = |side, pos| v.side_scroll(side, pos, 20.0);
    // 对齐点（视图中间）在删掉的那段里：左侧每次走一行，右侧不动。
    assert!((at(LEFT, 41.0) - at(LEFT, 40.0) - 1.0).abs() < 0.01);
    assert_eq!(at(RIGHT, 40.0), at(RIGHT, 41.0));
    assert!((at(RIGHT, 40.0) - 20.0).abs() < 0.01, "{}", at(RIGHT, 40.0));
    // 过了这段：两侧又一行对一行，右侧少 50 行。
    assert!((at(RIGHT, 90.0) - 40.0).abs() < 0.01);
    assert!((at(LEFT, 90.0) - 90.0).abs() < 0.01);
    // 换回来：右侧删除点在这段的开头，左侧的行原样。
    assert_eq!(v.pos_of(RIGHT, 30.0), 30.0);
    assert_eq!(v.pos_of(RIGHT, 40.0), 90.0);
    assert_eq!(v.pos_of(LEFT, 55.0), 55.0);
    // 屏幕第四行落在删掉的那段里，算这一段。
    assert_eq!(v.current_hunk(40.0), Some(0));
}

#[test]
fn a_long_deletion_at_the_end_can_still_be_scrolled_through() {
    // HEAD 末尾删掉了 80 行：右侧只有 20 行，滚动范围按左侧算，左侧能一直滚到末尾。
    let mut v = view(&numbered(100), &numbered(20));
    v.set_collapse(false, 0);
    assert_eq!(v.row_count(), 20);
    assert_eq!(v.scroll_rows(), 100);
    // 一屏 30 行、滚到底（末尾留四成）：左侧也停在末尾，最后一行在六成高的地方。
    let left = v.left_scroll(100.0 - 0.6 * 30.0, 30.0);
    assert!((left - 82.0).abs() < 0.01, "{left}");
    // 滚动一行，左侧也走一行（不按比例压缩成一次跳过去）。
    let a = v.left_scroll(40.0, 30.0);
    let b = v.left_scroll(41.0, 30.0);
    assert!((b - a - 1.0).abs() < 0.01, "{a} {b}");

    // 反过来：末尾新增的行在右侧，本来就能滚到，不多加。
    let v = view(&numbered(20), &numbered(100));
    assert_eq!(v.scroll_rows(), v.row_count());
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn each_side_lists_its_own_lines_without_padding() {
    let v = view("a\nb\nc\n", "a\nB\nX\nc\n");
    assert_eq!(
        v.rows[LEFT],
        [line(0, None), line(1, Some(0)), line(2, None)]
    );
    assert_eq!(
        v.rows[RIGHT],
        [
            line(0, None),
            line(1, Some(0)),
            line(2, Some(0)),
            line(3, None)
        ]
    );
    assert_eq!(v.hunk_rows, [vec![1..2], vec![1..3]]);
    assert_eq!(v.stats(), (2, 1));
    // 纯删除：右侧是插入位置上的空区间。
    let v = view("a\nb\nc\n", "a\nc\n");
    assert_eq!(v.hunk_rows, [vec![1..2], vec![1..1]]);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn long_unchanged_runs_fold_and_expand() {
    let old = numbered(40);
    let new = old.replace("line 20\n", "line twenty\n");
    let mut v = view(&old, &new);
    // 开头：折起 0..16，留 16..20；改动一行；结尾：留 21..25，折起 25..40。
    assert_eq!(
        v.rows[RIGHT][0],
        SideRow::Fold {
            gap: 0,
            lines: 0..16
        }
    );
    assert_eq!(v.row_count(), 1 + CONTEXT + 1 + CONTEXT + 1);
    assert_eq!(v.hunk_rows[RIGHT], [1 + CONTEXT..2 + CONTEXT]);

    v.expand(0);
    assert_eq!(v.rows[RIGHT][0], line(0, None));
    assert_eq!(v.hunk_rows[RIGHT], [20..21]);

    let row = v.set_collapse(false, 20);
    assert_eq!(v.row_count(), 40);
    assert_eq!(row, 20);
    assert!(!v.collapses());
}

#[test]
fn navigation_walks_hunks_in_order() {
    let old = numbered(30);
    let new = old
        .replace("line 2\n", "two\n")
        .replace("line 25\n", "twenty-five\n");
    let v = view(&old, &new);
    let first = v.next_hunk(0).unwrap();
    let second = v.next_hunk(first).unwrap();
    assert_eq!(v.next_hunk(second), None);
    assert_eq!(v.previous_hunk(second), Some(first));
    assert_eq!(v.previous_hunk(first), None);
    assert_eq!(v.hunk_at(second), Some(1));
}

#[test]
fn left_side_follows_the_right_around_hunks() {
    // 右侧在第 10 行插了 20 行。
    let old = numbered(60);
    let mut new = String::new();
    for (i, l) in old.lines().enumerate() {
        if i == 10 {
            new.push_str(&"added\n".repeat(20));
        }
        new.push_str(l);
        new.push('\n');
    }
    let mut v = view(&old, &new);
    v.set_collapse(false, 0);
    // 改动之前两侧一样；插入的那段滚过去时左侧停在插入点上；之后左侧少 20 行。
    assert_eq!(v.left_scroll(0.0, 10.0), 0.0);
    assert_eq!(v.left_scroll(2.0, 10.0), 2.0);
    assert_eq!(v.left_scroll(10.0, 10.0), 5.0);
    assert_eq!(v.left_scroll(40.0, 10.0), 20.0);
}

#[test]
fn pinned_lines_stay_visible_with_only_their_context() {
    let old = numbered(80);
    let mut v = view(&old, &old.replace("line 40\n", "forty\n"));
    // 改回去以后整个文件没有改动，不折叠；再改两处，中间那段很长。
    v.set_new_text(Rope::from_str(
        &old.replace("line 5\n", "five\n")
            .replace("line 75\n", "seventy-five\n"),
    ));
    v.pin_new(40..41);
    let row = v.row_of_new(40);
    assert_eq!(v.line_at(RIGHT, row), Some(40));
    // 钉住的那一行上下各露 CONTEXT 行，两边仍然折着。
    assert!(matches!(
        v.rows[RIGHT][row - CONTEXT - 1],
        SideRow::Fold { .. }
    ));
    assert!(matches!(
        v.rows[RIGHT][row + CONTEXT + 1],
        SideRow::Fold { .. }
    ));
    assert_eq!(v.line_at(LEFT, v.row_of(LEFT, 40)), Some(40));
}

#[test]
fn pinned_lines_follow_edits_above_them() {
    let old = numbered(80);
    let changed = old
        .replace("line 5\n", "five\n")
        .replace("line 75\n", "seventy-five\n");
    let mut v = view(&old, &changed);
    v.pin_new(40..41);
    // 在上面加两行：钉住的那一行跟着往下挪两行，仍然露在外面。
    let added = changed.replace("line 10\n", "line 10\nnew a\nnew b\n");
    v.set_new_text(Rope::from_str(&added));
    let row = v.row_of_new(42);
    assert_eq!(v.line_at(RIGHT, row), Some(42));
    assert!(matches!(
        v.rows[RIGHT][row + CONTEXT + 1],
        SideRow::Fold { .. }
    ));
}

#[test]
fn both_sides_start_at_the_top_when_the_file_starts_with_a_change() {
    // 开头一段左边 1 行、右边 5 行：右侧在开头时左侧也在开头。
    let old = numbered(60);
    let new = format!("a\nb\nc\nd\n{}", old.replacen("line 0\n", "zero\n", 1));
    let v = view(&old, &new);
    assert_eq!(v.left_scroll(0.0, 20.0), 0.0);
}

#[test]
fn identical_texts_have_no_hunks_and_no_folds() {
    let text = numbered(50);
    let v = view(&text, &text);
    assert!(v.is_identical());
    assert_eq!(v.row_count(), 50);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn inline_marks_split_per_line() {
    let v = view("let a = 1;\nlet b = 2;\n", "let a = 10;\nlet c = 2;\n");
    let marks = v.inline[0].as_ref().unwrap();
    assert_eq!(marks.old, [vec![8..9], vec![4..5]]);
    assert_eq!(marks.new, [vec![8..10], vec![4..5]]);
    // 纯新增没有逐词标记。
    let v = view("a\n", "a\nb\n");
    assert_eq!(v.inline, [None]);
}

#[test]
fn files_without_a_trailing_newline_line_up() {
    let v = view("a\nb", "a\nc");
    assert_eq!(v.row_count(), 2);
    assert_eq!(v.hunks.len(), 1);
}

#[test]
fn editing_the_right_side_rediffs_and_maps_lines() {
    let old = numbered(40);
    let mut v = view(&old, &old.replace("line 20\n", "line twenty\n"));
    assert_eq!(v.hunks.len(), 1);
    // 右侧第 2 行位于开头折起的一段里：先展开才看得见。
    assert!(matches!(
        v.rows[RIGHT][v.row_of_new(2)],
        SideRow::Fold { .. }
    ));
    v.reveal_new(2);
    assert_eq!(v.line_at(RIGHT, v.row_of_new(2)), Some(2));

    // 改回去：两侧相同。
    v.set_new_text(Rope::from_str(&old));
    assert!(v.is_identical());
    // 再改一处别的。
    v.set_new_text(Rope::from_str(&old.replace("line 3\n", "line three\n")));
    assert_eq!(v.hunks.len(), 1);
    assert_eq!(v.hunks[0].new, 3..4);
    assert_eq!(v.old_lines(0), "line 3\n");
}

#[test]
fn comparing_elsewhere_then_applying_matches_comparing_in_place() {
    let old = numbered(50);
    let new = old.replace("line 3\n", "three\n").replace("line 40\n", "");
    let mut a = view(&old, &old);
    let mut b = view(&old, &old);
    a.set_new_text(Rope::from_str(&new));
    let compared = Compared::new(&b.old.text, &Rope::from_str(&new));
    b.apply(Rope::from_str(&new), compared);
    assert_eq!(
        (a.hunks.clone(), a.keys.clone()),
        (b.hunks.clone(), b.keys.clone())
    );
    assert_eq!(a.rows, b.rows);
    // 左侧的选区：取出选中的文本。
    b.left_selection = Some((b.old.text.line_to_char(2), b.old.text.line_to_char(4)));
    assert_eq!(b.left_selected().as_deref(), Some("line 2\nline 3\n"));
}

#[test]
fn first_difference_counts_lines_across_chunks() {
    let a = Rope::from_str(&"汉字\n".repeat(2000));
    let mut b = a.clone();
    b.insert(a.line_to_char(1500) + 1, "x");
    assert_eq!(first_difference(&a, &b), 1500);
    assert_eq!(first_difference(&a, &a.clone()), a.len_lines() - 1);
}

mod measure {
    //! 重新比较耗时的测量：右侧每改一个字就重新比较一次。

    use super::*;

    /// 测量：右侧改一个字以后重新比较要多久。
    ///
    /// `cargo test -p winc-editor measure_rediff -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_rediff() {
        let language = winc_syntax::detect("a.rs");
        for n in [2_000, 10_000, 50_000] {
            let old: String = (0..n)
                .map(|i| format!("    let value_{i} = compute(value_{i}, \"text {i}\");\n"))
                .collect();
            let mut new = old.replace("value_7 ", "value_seven ");
            new = new.replace("value_1500 ", "value_x ");
            let mut v = DiffView::new(
                DiffSide::new("a", &old, None, language),
                DiffSide::new("b", &new, None, language),
            );
            let mut rope = Rope::from_str(&new);
            let start = std::time::Instant::now();
            for i in 0..20 {
                rope.insert(
                    rope.line_to_char(n / 2) + 4,
                    if i % 2 == 0 { "x" } else { "y" },
                );
                v.set_new_text(rope.clone());
            }
            eprintln!("{n} 行：{:?} / 次", start.elapsed() / 20);
        }
    }
}
