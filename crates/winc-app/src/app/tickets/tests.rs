//! 后台任务代号的测试：每个键只收最新一次的结果。

use super::*;

#[test]
fn only_the_latest_ticket_of_a_key_is_redeemed() {
    let mut tickets = Tickets::default();
    let first = tickets.issue("a");
    let other = tickets.issue("b");
    let second = tickets.issue("a");

    // 先发的晚到：作废，仍然等后发的那次。
    assert!(!tickets.redeem(&"a", first));
    assert!(tickets.redeem(&"a", second));
    // 收过的不能再收。
    assert!(!tickets.redeem(&"a", second));
    // 各键互不影响。
    assert!(tickets.redeem(&"b", other));
}
