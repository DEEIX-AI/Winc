//! 关闭流程的测试：提示框按钮的序号与保存选择的对应。

use super::*;

#[test]
fn buttons_map_to_choices_in_macos_order() {
    assert_eq!(SaveChoice::from_index(0), SaveChoice::Save);
    assert_eq!(SaveChoice::from_index(1), SaveChoice::Cancel);
    assert_eq!(SaveChoice::from_index(2), SaveChoice::Discard);
    assert_eq!(SaveChoice::from_index(9), SaveChoice::Cancel);
}
