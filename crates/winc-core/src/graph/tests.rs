//! `graph` 模块的测试：直线历史、分支与合并、多父提交、车道复用与分批推算的一致性。

use super::*;

/// 按给定顺序把 `(oid, 父提交)` 逐个推入新图，返回每行的画法。
fn rows(commits: &[(&str, &[&str])]) -> Vec<GraphRow> {
    let mut graph = Graph::new();
    commits
        .iter()
        .map(|(oid, parents)| graph.push(oid, parents))
        .collect()
}

/// 返回以起止车道表示的线段，便于在断言中写出期望值。
fn seg(from: usize, to: usize) -> (usize, usize) {
    (from, to)
}

/// 返回按起止车道排序的线段，使断言的结果与 `push` 中的添加顺序无关。
fn lines(segments: &[Segment]) -> Vec<(usize, usize)> {
    let mut out: Vec<_> = segments.iter().map(|s| (s.from, s.to)).collect();
    out.sort_unstable();
    out
}

#[test]
fn a_straight_history_stays_on_one_lane() {
    let rows = rows(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
    assert!(rows.iter().all(|r| r.lane == 0 && r.width == 1));
    assert!(rows[0].up.is_empty());
    assert_eq!(lines(&rows[0].down), [seg(0, 0)]);
    assert_eq!(lines(&rows[1].up), [seg(0, 0)]);
    // 根提交下面没有线。
    assert!(rows[2].down.is_empty());
    assert_eq!(rows[0].color, rows[2].color);
}

#[test]
fn a_branch_and_its_merge_take_a_second_lane() {
    // `m` 把分支 `f` 合并进主线 `c`；`f` 和 `c` 都从 `b` 分出来。
    let rows = rows(&[("m", &["c", "f"]), ("f", &["b"]), ("c", &["b"]), ("b", &[])]);
    // 合并提交：圆点在 0 号车道，下半行分成两条。
    assert_eq!(rows[0].lane, 0);
    assert_eq!(lines(&rows[0].down), [seg(0, 0), seg(0, 1)]);
    // `f` 在 1 号车道，颜色与主线不同；主线直着穿过。
    assert_eq!(rows[1].lane, 1);
    assert_ne!(rows[1].color, rows[0].color);
    assert_eq!(lines(&rows[1].up), [seg(0, 0), seg(1, 1)]);
    // `c` 在主线上；`f` 那条线还等着 `b`，直着穿过。
    assert_eq!(rows[2].lane, 0);
    assert_eq!(lines(&rows[2].down), [seg(0, 0), seg(1, 1)]);
    // `b`：两条线在这里汇合，汇合以后只剩一条。
    assert_eq!(rows[3].lane, 0);
    assert_eq!(lines(&rows[3].up), [seg(0, 0), seg(1, 0)]);
    assert!(rows[3].down.is_empty());
    assert_eq!(rows[3].width, 2);
}

#[test]
fn two_unrelated_roots_sit_side_by_side() {
    let rows = rows(&[("x", &["a"]), ("y", &["b"]), ("a", &[]), ("b", &[])]);
    assert_eq!((rows[0].lane, rows[1].lane), (0, 1));
    assert_eq!(lines(&rows[1].up), [seg(0, 0)]);
    assert_eq!(rows[2].lane, 0);
    // `a` 是根提交：0 号车道空出来以后，`b` 仍在 1 号车道上。
    assert_eq!(rows[3].lane, 1);
    assert_eq!(lines(&rows[3].up), [seg(1, 1)]);
}

#[test]
fn an_octopus_merge_fans_out_to_every_parent() {
    let rows = rows(&[
        ("m", &["a", "b", "c"]),
        ("a", &["r"]),
        ("b", &["r"]),
        ("c", &["r"]),
        ("r", &[]),
    ]);
    assert_eq!(lines(&rows[0].down), [seg(0, 0), seg(0, 1), seg(0, 2)]);
    assert_eq!((rows[1].lane, rows[2].lane, rows[3].lane), (0, 1, 2));
    assert_eq!(lines(&rows[4].up), [seg(0, 0), seg(1, 0), seg(2, 0)]);
}

#[test]
fn a_merge_into_a_lane_that_is_already_waiting_reuses_it() {
    // 两个合并提交都并入 `f`：第二个不再开新车道，而是连到已经等着 `f` 的那条。
    let rows = rows(&[
        ("m1", &["a", "f"]),
        ("m2", &["b", "f"]),
        ("a", &["r"]),
        ("b", &["r"]),
        ("f", &["r"]),
        ("r", &[]),
    ]);
    assert_eq!(rows[1].lane, 2);
    assert!(rows[1].down.iter().any(|s| s.from == 2 && s.to == 1));
    assert!(rows.iter().all(|r| r.width <= 3));
}

#[test]
fn each_line_keeps_its_own_number_even_when_colors_repeat() {
    // `m` 合并了 `f`：`f` 那条线与主线编号不同，主线自始至终是同一个编号。
    let rows = rows(&[("m", &["c", "f"]), ("f", &["b"]), ("c", &["b"]), ("b", &[])]);
    let main = rows[0].line;
    assert_eq!((rows[2].line, rows[3].line), (main, main));
    assert_ne!(rows[1].line, main);
    // 合并提交的下半行：连主线的那段属于主线，连 `f` 的那段属于 `f` 的线。
    let to_f = rows[0].down.iter().find(|s| s.to == 1).unwrap();
    assert_eq!(to_f.line, rows[1].line);
    // `f` 合回 `b` 的那段仍属于 `f` 的线。
    let back = rows[3].up.iter().find(|s| s.from == 1).unwrap();
    assert_eq!(back.line, rows[1].line);

    // 分支数超过颜色数：颜色轮流重复使用，编号不重复。
    let tips: Vec<String> = (0..COLORS as usize + 2).map(|i| format!("t{i}")).collect();
    let mut graph = Graph::new();
    let lines: Vec<u32> = tips.iter().map(|t| graph.push(t, &["r"]).line).collect();
    let mut unique = lines.clone();
    unique.dedup();
    assert_eq!(unique.len(), lines.len());
}

#[test]
fn batches_continue_where_the_last_one_stopped() {
    let all = [("c", &["b"][..]), ("b", &["a"][..]), ("a", &[][..])];
    let mut whole = Graph::new();
    let expected: Vec<GraphRow> = all.iter().map(|(o, p)| whole.push(o, p)).collect();
    let mut graph = Graph::new();
    let mut got: Vec<GraphRow> = all[..2].iter().map(|(o, p)| graph.push(o, p)).collect();
    got.extend(all[2..].iter().map(|(o, p)| graph.push(o, p)));
    assert_eq!(got, expected);
}
