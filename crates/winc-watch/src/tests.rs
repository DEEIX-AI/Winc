//! `winc-watch` 的测试：把通知编码成消息。

use super::*;

use notify::event::{AccessKind, CreateKind, Flag};
use notify::{Error, Event};

fn event(kind: EventKind, paths: &[&str]) -> Event {
    paths.iter().fold(Event::new(kind), |event, path| {
        event.add_path(PathBuf::from(path))
    })
}

#[test]
fn each_changed_path_is_one_message() {
    let created = event(EventKind::Create(CreateKind::File), &["/a b/c", "/d"]);
    assert_eq!(encode(&Ok(created)), b"P/a b/c\0P/d\0");
}

#[test]
fn reading_a_file_is_not_a_change() {
    let read = event(EventKind::Access(AccessKind::Any), &["/a"]);
    assert!(encode(&Ok(read)).is_empty());
}

#[test]
fn lost_notifications_and_errors_ask_for_a_rescan() {
    let lost = Event::new(EventKind::Other).set_flag(Flag::Rescan);
    assert_eq!(encode(&Ok(lost)), b"R\0");
    assert_eq!(encode(&Err(Error::generic("x"))), b"R\0");
}
