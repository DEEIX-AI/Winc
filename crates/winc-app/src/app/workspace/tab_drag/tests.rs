//! `tab_drag` 模块的测试：关闭标签后右侧标签滑回的起点偏移。

use super::close_shifts;
use crate::theme::TAB_GAP;

#[test]
fn tabs_right_of_a_closed_one_slide_back_by_its_width() {
    let widths = [100.0, 80.0, 120.0, 90.0];
    // 关掉第 1 个：右边两个都从它的宽度加间距的距离滑回来。
    let step = 80.0 + TAB_GAP;
    assert_eq!(close_shifts(&widths, 1, None), [(2, step), (3, step)]);
    // 用鼠标关（紧邻的那个暂用被关标签的宽度）：它不动，再往右的滑回它自己原来的宽度。
    assert_eq!(
        close_shifts(&widths, 1, Some((1, 80.0))),
        [(2, step), (3, 120.0 + TAB_GAP)]
    );
    // 关掉最后一个：没有要滑的。
    assert!(close_shifts(&widths, 3, None).is_empty());
}
