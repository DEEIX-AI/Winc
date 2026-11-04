//! `selection` 模块的测试：多选区的排序、合并规则与合并后保留的方向。

use super::*;

#[test]
fn merging_sorts_and_joins_overlapping_selections() {
    // 光标重合、光标落在选区内都合并；主选区随所在的选区一起并入。
    let (merged, primary) = merge(
        vec![
            Selection::point(9),
            Selection::new(2, 5),
            Selection::point(4),
            Selection::point(9),
        ],
        2,
    );
    assert_eq!(merged, vec![Selection::new(2, 5), Selection::point(9)]);
    assert_eq!(primary, 0);

    // 两个非空选区首尾相接时不合并；光标贴着选区的一端时与它合并。
    let (merged, primary) = merge(
        vec![
            Selection::new(0, 3),
            Selection::new(3, 6),
            Selection::point(6),
        ],
        1,
    );
    assert_eq!(merged, vec![Selection::new(0, 3), Selection::new(3, 6)]);
    assert_eq!(primary, 1);

    // 合并后沿用主选区的方向：主选区反向时，合并结果也反向。
    let (merged, _) = merge(vec![Selection::new(0, 4), Selection::new(6, 2)], 1);
    assert_eq!(merged, vec![Selection::new(6, 0)]);
    assert_eq!(merge(Vec::new(), 0), (Vec::new(), 0));
}
