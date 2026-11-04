//! 标签栏的测试：标签放不下时从最宽的开始变窄，且不窄于最小宽度。

use super::*;

#[test]
fn crowded_tabs_shrink_the_widest_first() {
    let mut widths = [120.0, 200.0, 120.0];
    fit_tabs(&mut widths, 1000.0);
    assert_eq!(widths, [120.0, 200.0, 120.0]);
    // 放 400：宽的先窄到和别的一样，再一起窄。
    fit_tabs(&mut widths, 400.0 + 2.0 * TAB_GAP);
    assert_eq!(widths, [120.0, 160.0, 120.0]);
    fit_tabs(&mut widths, 300.0 + 2.0 * TAB_GAP);
    assert_eq!(widths, [100.0, 100.0, 100.0]);
    // 再窄也不窄过最窄的宽度。
    fit_tabs(&mut widths, 30.0);
    assert_eq!(widths, [TAB_SQUEEZED; 3]);
}
