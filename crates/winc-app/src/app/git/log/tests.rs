//! 日志模块的测试：提交时间的显示和本地时间的换算。

use super::*;

/// 日志标签的标题，测试中与 [`log_title`] 在默认语言下的结果比较。
#[cfg(unix)]
pub(crate) const LOG_TITLE: &str = "Git 日志";

fn commit(local: &str, time: i64) -> Commit {
    Commit {
        oid: "a".into(),
        parents: Vec::new(),
        author: String::new(),
        email: String::new(),
        time,
        local: local.into(),
        refs: Vec::new(),
        subject: String::new(),
    }
}

#[test]
fn times_read_like_a_person_would_say_them() {
    // 东八区：本地 2026-10-26 14:32 是 UTC 06:32。
    let at = naive_seconds("2026-10-26 14:32").unwrap() - 8 * 3600;
    let c = commit("2026-10-26 14:32", at);
    assert_eq!(display_time(&c, at + 30), "刚刚");
    assert_eq!(display_time(&c, at + 25 * 60), "25 分钟前");
    assert_eq!(display_time(&c, at + 3 * 3600), "今天 14:32");
    // 第二天早上（本地 10-27 08:00）。
    assert_eq!(display_time(&c, at + 17 * 3600 + 28 * 60), "昨天 14:32");
    assert_eq!(display_time(&c, at + 30 * 86_400), "10月26日");
    assert_eq!(display_time(&c, at + 120 * 86_400), "2026年10月26日");
    // 列表里一种全是数字的写法：今年的「月-日 时:分」（补零），更早的「年-月-日」；不写
    // 「今天」「几分钟前」。
    assert_eq!(list_time(&c, at + 30 * 86_400), "10-26 14:32");
    assert_eq!(list_time(&c, at + 3 * 3600), "10-26 14:32");
    assert_eq!(list_time(&c, at + 30), "10-26 14:32");
    assert_eq!(list_time(&c, at + 120 * 86_400), "2026-10-26");
    let early = commit("2026-09-03 18:22", at - 53 * 86_400);
    assert_eq!(list_time(&early, at), "09-03 18:22");
}

#[test]
fn local_text_converts_with_the_offset() {
    let at = naive_seconds("2026-10-26 14:32").unwrap() - 8 * 3600;
    assert_eq!(local_text(at, 8 * 3600), "2026-10-26 14:32");
    assert_eq!(local_text(at, 0), "2026-10-26 06:32");
    assert_eq!(civil_from_days(days_from_civil(2000, 2, 29)), (2000, 2, 29));
}

#[test]
fn civil_dates_round_trip() {
    for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 10, 26), (2026, 12, 31)] {
        assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
    }
    assert_eq!(days_from_civil(1970, 1, 2), 1);
}
