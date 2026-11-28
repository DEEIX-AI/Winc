use ropey::Rope;

use super::*;

fn view(old: &str, new: &str) -> DiffView {
    let language = winc_syntax::detect("a.rs");
    let mut view = DiffView::new(
        DiffSide::new("old", old, None, language),
        DiffSide::new("new", new, None, language),
    );
    view.set_collapse(false, 0);
    view
}

/// 一段 20 行的函数，之后是一行未改动的函数和一行改动。
fn long_function() -> (String, String) {
    let mut old = String::from("fn a() {\n");
    for i in 0..20 {
        old.push_str(&format!("    s{i}();\n"));
    }
    old.push_str("}\nfn tail() {}\nlast_old\n");
    let new = old.replace("last_old", "last_new");
    (old, new)
}

/// 同一个滚动位置两侧显示的行。
fn shown(v: &DiffView, pos: f64) -> [Option<usize>; 2] {
    [LEFT, RIGHT].map(|side| v.line_at(side, v.row_at(side, pos) as usize))
}

#[test]
fn code_blocks_fold_inside_a_diff_and_reveal_on_navigation() {
    let text =
        "fn main() {\n    if ready {\n        one();\n        two();\n    }\n    tail();\n}\n";
    let mut v = view(text, &text.replace("two();", "changed();"));

    let (row, changed, visible) = v.run_code_fold((RIGHT, 2), FoldCommand::Fold, 0, 4);
    assert!(changed);
    assert_eq!(row, 0);
    assert_eq!(visible, 1);
    assert_eq!(v.collapsed_code_fold(RIGHT, 1), Some(2..4));
    assert_eq!(v.visible_code_line(RIGHT, 3), 1);
    assert_eq!(v.line_at(RIGHT, v.row_of(RIGHT, 1)), Some(1));
    assert!(
        !v.rows[RIGHT]
            .iter()
            .any(|row| matches!(row, SideRow::Line { line: 2..=3, .. }))
    );

    // 查找、跳转等定位到折叠内部时只展开包住目标的块。
    v.reveal_line(RIGHT, 3);
    assert_eq!(v.collapsed_code_fold(RIGHT, 1), None);
    assert_eq!(v.line_at(RIGHT, v.row_of(RIGHT, 3)), Some(3));
}

#[test]
fn folding_one_side_hides_the_same_unchanged_lines_on_the_other() {
    let (old, new) = long_function();
    let mut v = view(&old, &new);
    let (_, changed, _) = v.run_code_fold((LEFT, 1), FoldCommand::Fold, 0, 4);
    assert!(changed);
    // 两侧隐藏的是同样的 20 行（右花括号仍显示），收起标记在两侧同一行。
    assert_eq!(v.collapsed_code_fold(LEFT, 0), Some(1..21));
    assert_eq!(v.collapsed_code_fold(RIGHT, 0), Some(1..21));
    assert_eq!(v.rows[LEFT].len(), v.rows[RIGHT].len());
    // 滚到任何位置，两侧显示的都是同一行。
    for pos in 0..v.rows[RIGHT].len() {
        let [left, right] = shown(&v, pos as f64);
        assert_eq!(left, right, "滚动位置 {pos}");
    }
}

#[test]
fn a_fold_around_a_change_keeps_both_sides_aligned() {
    // 块里有一处改动：左侧一行换成右侧三行。
    let old = "fn a() {\n    one();\n    two();\n    three();\n}\ntail();\n";
    let new = "fn a() {\n    one();\n    x();\n    y();\n    z();\n    three();\n}\ntail();\n";
    let mut v = view(old, new);
    v.run_code_fold((RIGHT, 1), FoldCommand::Fold, 0, 4);
    // 改动整个在块内：两侧都隐藏块体，包括改动的行。
    assert_eq!(v.collapsed_code_fold(RIGHT, 0), Some(1..6));
    assert_eq!(v.collapsed_code_fold(LEFT, 0), Some(1..4));
    // 块之后的内容两侧仍然对齐。
    let last = v.rows[RIGHT].len() - 1;
    assert_eq!(shown(&v, v.pos_of(RIGHT, last as f64)), [Some(5), Some(7)]);
}

#[test]
fn boundaries_inside_a_change_keep_the_other_side_of_it() {
    // 左侧第 1 行换成右侧第 1 到 3 行。
    let v = view("a\nb\nc\n", "a\nX\nY\nZ\nc\n");
    // 落在改动中间的起点取另一侧改动之后，终点取改动之前：另一侧不隐藏无法对应的行。
    assert_eq!(v.map_boundary(RIGHT, 2, Edge::Start), 2);
    assert_eq!(v.map_boundary(RIGHT, 2, Edge::End), 1);
    // 改动前后的未改动内容逐行对应。
    assert_eq!(v.map_boundary(RIGHT, 1, Edge::Start), 1);
    assert_eq!(v.map_boundary(RIGHT, 4, Edge::End), 2);
    assert_eq!(v.map_boundary(LEFT, 3, Edge::End), 5);
}

#[test]
fn the_other_side_unfolds_a_mirrored_fold() {
    let (old, new) = long_function();
    let mut v = view(&old, &new);
    v.run_code_fold((LEFT, 1), FoldCommand::Fold, 0, 4);
    // 右侧没有自己的块收起，在收起标记所在的行展开的是左侧的块。
    let (_, changed, _) = v.run_code_fold((RIGHT, 0), FoldCommand::Unfold, 0, 4);
    assert!(changed);
    assert_eq!(v.collapsed_code_fold(LEFT, 0), None);
    // 点击右侧的收起标记同样展开，再点一次收起右侧以该行开头的块。
    v.run_code_fold((LEFT, 1), FoldCommand::Fold, 0, 4);
    v.toggle_code_fold_at((RIGHT, 0), 0, 4);
    assert_eq!(v.collapsed_code_fold(RIGHT, 0), None);
    v.toggle_code_fold_at((RIGHT, 0), 0, 4);
    assert_eq!(v.collapsed_code_fold(LEFT, 0), Some(1..21));
}

#[test]
fn fold_all_folds_one_side_and_unfold_all_clears_both() {
    let text = "fn main() {\n    if ready {\n        one();\n    }\n}\nfn b() {\n    x();\n}\n";
    let mut v = view(text, text);
    v.run_code_fold((LEFT, 0), FoldCommand::FoldAll, 0, 4);
    v.run_code_fold((RIGHT, 5), FoldCommand::Fold, 0, 4);
    assert!(v.code_folds.iter().any(|fold| fold.side == LEFT));
    assert!(v.code_folds.iter().any(|fold| fold.side == RIGHT));
    let (_, changed, _) = v.run_code_fold((LEFT, 0), FoldCommand::UnfoldAll, 0, 4);
    assert!(changed);
    assert!(v.code_folds.is_empty());
    assert_eq!(v.rows[LEFT].len(), 8);
}

#[test]
fn folds_follow_edits_above_them_and_drop_when_edited_inside() {
    let text = "fn a() {\n    x();\n}\nfn b() {\n    y();\n}\n";
    let mut v = view(text, text);
    v.run_code_fold((RIGHT, 3), FoldCommand::Fold, 0, 4);
    // 上面多出一行：块跟着下移，仍然收起。
    v.set_new_text(Rope::from_str(&format!("// new\n{text}")));
    assert!(v.collapsed_code_fold(RIGHT, 4).is_some());
    // 在块里修改：这个折叠不再保留。
    v.set_new_text(Rope::from_str(&format!(
        "// new\n{}",
        text.replace("y();", "z();")
    )));
    assert!(v.code_folds.is_empty());
}
