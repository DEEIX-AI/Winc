//! `sidebar` 的测试：合并显示的目录放不下时的折叠。

use super::*;

/// 十级、每级宽 40、分隔符宽 10、折叠记号宽 10：全部显示共宽 490。
fn folded(current: usize, room: f32) -> (Vec<Option<usize>>, f32) {
    fold_chain(&[40.0; 10], 10.0, 10.0, current, room)
}

#[test]
fn a_chain_that_fits_is_not_folded() {
    let (parts, width) = folded(9, 490.0);
    assert_eq!(parts, (0..10).map(Some).collect::<Vec<_>>());
    assert_eq!(width, 490.0);
}

#[test]
fn the_first_current_and_last_levels_stay_visible() {
    // 当前在中间，放得下第一级、当前、最后一级和两处折叠。
    let (parts, width) = folded(5, 180.0);
    assert_eq!(parts, [Some(0), None, Some(5), None, Some(9)]);
    assert!(width <= 180.0);

    // 当前是最后一级或第一级时只有一处折叠。
    assert_eq!(folded(9, 120.0).0, [Some(0), None, Some(9)]);
    assert_eq!(folded(0, 120.0).0, [Some(0), None, Some(9)]);
}

#[test]
fn spare_room_goes_to_the_levels_around_the_current_one() {
    // 多出一级的空间，距离相同时补靠后的一级；再多一级补靠前的。
    assert_eq!(
        folded(5, 230.0).0,
        [Some(0), None, Some(5), Some(6), None, Some(9)]
    );
    assert_eq!(
        folded(5, 280.0).0,
        [Some(0), None, Some(4), Some(5), Some(6), None, Some(9)]
    );
}

#[test]
fn the_last_level_gives_way_when_room_is_very_tight() {
    // 放不下三级时保留第一级和当前的一级。
    assert_eq!(folded(5, 120.0).0, [Some(0), None, Some(5), None]);
    // 再窄也不丢掉这两级，由绘制时裁掉放不下的部分。
    assert_eq!(folded(5, 10.0).0, [Some(0), None, Some(5), None]);
}
