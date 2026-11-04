//! 命令块的测试：时长与命令名的写法、重新运行命令时发给 shell 的字节。

use super::*;

#[test]
fn durations_read_naturally() {
    assert_eq!(format_duration(800), "0.8 s");
    assert_eq!(format_duration(12_400), "12.4 s");
    assert_eq!(format_duration(185_000), "3 min 5 s");
    assert_eq!(format_duration(3_720_000), "1 h 2 min");
}

#[test]
fn long_command_lines_are_cut_in_notices() {
    let block = |text: &str| Block {
        text: text.into(),
        ..Block::default()
    };
    assert_eq!(command_name(&block("cargo build")), "「cargo build」");
    assert_eq!(command_name(&block("")), "命令");
    assert!(command_name(&block(&"x".repeat(60))).ends_with("…」"));
}

#[test]
fn reruns_clear_the_line_and_press_return() {
    assert_eq!(rerun_bytes("ls -l", Modes::default()), b"\x15ls -l\r");
    let bracketed = rerun_bytes("ls", Modes(Modes::BRACKETED_PASTE));
    assert_eq!(bracketed, b"\x15\x1b[200~ls\x1b[201~\r");
}
