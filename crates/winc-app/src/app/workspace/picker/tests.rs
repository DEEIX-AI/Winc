//! `picker` 模块的测试：快速打开查询中 `:行:列` 后缀的拆分。

use super::*;

#[test]
fn quick_open_queries_split_off_line_numbers() {
    assert_eq!(split_goto("main.rs"), ("main.rs", None));
    assert_eq!(split_goto("main.rs:42"), ("main.rs", Some("42")));
    assert_eq!(split_goto("main.rs:42:7"), ("main.rs", Some("42:7")));
    assert_eq!(split_goto(":42"), ("", Some("42")));
    assert_eq!(split_goto("a:b"), ("a:b", None));
    assert_eq!(split_goto("main.rs:"), ("main.rs:", None));
}
