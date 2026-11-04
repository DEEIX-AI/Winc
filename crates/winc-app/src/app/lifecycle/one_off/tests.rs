//! 临时打开模式的测试：关闭标准输入标签后停止读取。

use super::*;
use crate::bench::Bench;

/// 测试中标准输入标签的标题：测试以源语言运行，与 [`stdin_title`] 的结果相同。
const STDIN_TITLE: &str = "标准输入";

#[test]
fn closing_the_stdin_tab_stops_reading() {
    let mut app = App::new(
        Editor::default(),
        None,
        Bench::from_env(),
        Config::default(),
    );
    let index = app.editor.show_output(STDIN_TITLE, "");
    let id = app.editor.documents[index].id;
    let (send, receive) = mpsc::channel();
    app.one_off.stdin = Some((id, receive));

    send.send(Piece::Text("one\n".to_owned())).unwrap();
    app.poll_stdin();
    assert_eq!(app.editor.documents[0].text().line_count(), 2);

    // 关闭标签后 `poll_stdin` 丢弃接收端，发送端再发送会返回错误，后台线程据此退出。
    app.remove_tabs(&[id]);
    let _ = send.send(Piece::Text("two\n".to_owned()));
    app.poll_stdin();
    assert!(app.one_off.stdin.is_none());
    assert!(send.send(Piece::Text("three\n".to_owned())).is_err());
}
