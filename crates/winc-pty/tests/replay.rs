//! 回放录制的真实程序输出（全屏编辑器、分页器、`git log --graph`、`ls`），与存好的屏幕
//! 快照比对。
//!
//! 录制环境：80 × 24、`TERM=xterm-256color`，终端不回答查询。快照含屏幕文字、光标位置、
//! 是否在备用屏和几行的样式段。确认新结果正确后，用
//! `WINC_UPDATE_FIXTURES=1 cargo test -p winc-pty --test replay` 重新生成。

use std::fmt::Write as _;
use std::path::Path;

use winc_pty::Terminal;
use winc_pty::protocol::Modes;

fn snapshot(t: &Terminal, styled_rows: &[usize]) -> String {
    let mut out = String::new();
    for r in 0..t.rows() {
        let _ = writeln!(out, "{:2}|{}", r, t.line(r).trimmed());
    }
    let (row, col) = t.cursor();
    let _ = writeln!(out, "cursor {row},{col}");
    let _ = writeln!(out, "alternate {}", t.modes().has(Modes::ALT_SCREEN));
    for &r in styled_rows {
        let line = t.line(r);
        let _ = writeln!(out, "row {r} runs:");
        for run in &line.runs {
            let _ = writeln!(
                out,
                "  {:3} fg={:?} bg={:?} attrs={:#x}",
                run.start, run.style.fg, run.style.bg, run.style.attrs.0
            );
        }
    }
    out
}

fn replay(name: &str, styled_rows: &[usize]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let input = std::fs::read(dir.join(format!("{name}.out"))).unwrap();

    // 整段喂入与逐字节喂入的结果必须相同：序列被切分在两个读取块之间时不能出错。
    let mut whole = Terminal::new(80, 24, 1000);
    whole.feed(&input);
    let mut bytewise = Terminal::new(80, 24, 1000);
    for byte in &input {
        bytewise.feed(std::slice::from_ref(byte));
    }
    let actual = snapshot(&whole, styled_rows);
    assert_eq!(
        actual,
        snapshot(&bytewise, styled_rows),
        "{name}：分开喂的结果不一样"
    );

    let expected_path = dir.join(format!("{name}.screen"));
    if std::env::var_os("WINC_UPDATE_FIXTURES").is_some() {
        std::fs::write(&expected_path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|_| panic!("没有 {name}.screen，先用 WINC_UPDATE_FIXTURES=1 生成"));
    assert_eq!(actual, expected, "{name} 的屏幕和快照不一样");
}

#[test]
fn vim_editing_a_file() {
    replay("vim", &[0, 2]);
}

#[test]
fn vim_quits_back_to_the_shell_screen() {
    replay("vim-quit", &[]);
}

#[test]
fn less_after_a_search() {
    replay("less", &[0, 23]);
}

#[test]
fn git_log_graph_in_colour() {
    replay("git-log", &[0, 1]);
}

#[test]
fn ls_in_colour() {
    replay("ls", &[0]);
}
