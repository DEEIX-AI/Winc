//! 逐行追溯的测试：改动前后的行沿袭，改过的行记为未提交。

use ropey::Rope;
use winc_editor::git::{Blame, BlameCommit};

use super::shifted;

/// 三次提交，各管一行。
fn blame() -> Blame {
    let commit = |summary: &str| BlameCommit {
        summary: summary.into(),
        ..BlameCommit::default()
    };
    Blame {
        commits: vec![commit("a"), commit("b"), commit("c")],
        lines: vec![0, 1, 2],
    }
}

fn summaries(blame: &Blame) -> Vec<String> {
    let name = |&i: &u32| {
        let c = &blame.commits[i as usize];
        if c.uncommitted {
            "?".into()
        } else {
            c.summary.clone()
        }
    };
    blame.lines.iter().map(name).collect()
}

#[test]
fn shifting_keeps_untouched_lines_and_marks_edits_uncommitted() {
    let old = Rope::from_str("one\ntwo\nthree\n");
    let cases = [
        ("one\nnew\ntwo\nthree\n", vec!["a", "?", "b", "c"]),
        ("one\ntwo!\nthree\n", vec!["a", "?", "c"]),
        ("one\nthree\n", vec!["a", "c"]),
        ("zero\none\ntwo\nthree\n", vec!["?", "a", "b", "c"]),
        ("one\ntwo\nthree\nfour\n", vec!["a", "b", "c", "?"]),
        ("", vec![]),
    ];
    for (new, want) in cases {
        let got = shifted(&blame(), &old, &Rope::from_str(new));
        assert_eq!(summaries(&got), want, "{new:?}");
    }
}

#[test]
fn shifting_reuses_the_uncommitted_commit() {
    let old = Rope::from_str("one\ntwo\nthree\n");
    let once = shifted(&blame(), &old, &Rope::from_str("one\nx\nthree\n"));
    let twice = shifted(
        &once,
        &Rope::from_str("one\nx\nthree\n"),
        &Rope::from_str("one\ny\nthree\n"),
    );
    assert_eq!(twice.commits.len(), once.commits.len());
}
