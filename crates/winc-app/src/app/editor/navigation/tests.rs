//! 后退、前进历史的测试：哪些移动算跳转、新跳转丢弃可前进的项、条目上限。

use super::*;

fn spot(id: u64, line: usize) -> Spot {
    Spot {
        id: DocumentId(id),
        path: None,
        line,
        col: 0,
    }
}

fn lines(nav: &Navigation) -> Vec<(u64, usize)> {
    nav.spots.iter().map(|s| (s.id.0, s.line)).collect()
}

#[test]
fn only_jumps_add_a_spot() {
    let mut nav = Navigation::default();
    nav.observe(spot(1, 0), 0);
    // 移动几行（不超过 `FAR`）只更新当前项，不新增。
    nav.observe(spot(1, 5), 0);
    assert_eq!(lines(&nav), [(1, 5)]);
    // 移动超过 `FAR` 行、或换了文件，各新增一项。
    nav.observe(spot(1, 40), 0);
    nav.observe(spot(2, 3), 0);
    assert_eq!(lines(&nav), [(1, 5), (1, 40), (2, 3)]);
    assert_eq!(nav.current, 2);

    // 编辑把光标带到很远的位置（例如粘贴一大段）不算跳转。
    nav.observe(spot(2, 300), 1);
    assert_eq!(lines(&nav), [(1, 5), (1, 40), (2, 300)]);
    // 编辑之后再次跳转仍然算跳转。
    nav.observe(spot(2, 3), 1);
    assert_eq!(nav.spots.len(), 4);
}

#[test]
fn a_new_jump_after_going_back_drops_the_forward_spots() {
    let mut nav = Navigation::default();
    for line in [0, 100, 200, 300] {
        nav.observe(spot(1, line), 0);
    }
    nav.current = 1;
    nav.observe(spot(1, 102), 0);
    assert_eq!(lines(&nav), [(1, 0), (1, 102), (1, 200), (1, 300)]);
    assert!(nav.can_go(true));

    nav.observe(spot(1, 500), 0);
    assert_eq!(lines(&nav), [(1, 0), (1, 102), (1, 500)]);
    assert!(!nav.can_go(true));
}

#[test]
fn only_the_latest_spots_are_kept() {
    let mut nav = Navigation::default();
    for i in 0..LIMIT + 10 {
        nav.observe(spot(i as u64, 0), 0);
    }
    assert_eq!(nav.spots.len(), LIMIT);
    assert_eq!(nav.spots[0].id, DocumentId(10));
    assert_eq!(nav.current, LIMIT - 1);
}
