//! `tool_fold` 模块的测试：工具栏按钮收起与展开时逐帧推进的程度。

use super::*;

impl ToolFold {
    /// 停在给定的程度上、不再推进的状态，仅用于测试截图。
    pub fn frozen(tools: [f32; TOOLS], more: f32) -> ToolFold {
        let mut shown = [0.0; TOOLS + 1];
        shown[..TOOLS].copy_from_slice(&tools);
        shown[TOOLS] = more;
        ToolFold {
            shown,
            target: shown,
            at: None,
            placed: true,
            frozen: true,
        }
    }
}

#[test]
fn folding_moves_at_a_steady_pace_and_turns_back_smoothly() {
    let mut fold = ToolFold {
        shown: [1.0; TOOLS + 1],
        target: [1.0, 1.0, 1.0, 0.0, 0.0, 1.0],
        ..ToolFold::default()
    };
    fold.shown[TOOLS] = 0.0;

    // 走了一半时间：要收的按钮收了一半，「⋯」出来一半，其余不动。
    fold.advance(TOOL_FOLD / 2);
    assert_eq!(fold.shown, [1.0, 1.0, 1.0, 0.5, 0.5, 0.5]);
    assert!(fold.moving());

    // 途中又变宽：从一半的地方往回走，不跳。
    fold.target = [1.0; TOOLS + 1];
    fold.target[TOOLS] = 0.0;
    fold.advance(TOOL_FOLD / 4);
    assert_eq!(fold.shown, [1.0, 1.0, 1.0, 0.75, 0.75, 0.25]);

    // 走过头也停在目标上。
    fold.advance(TOOL_FOLD);
    assert_eq!(fold.shown, fold.target);
    assert!(!fold.moving());

    // 缓动不改变两端。
    let (tools, more) = fold.eased();
    assert_eq!(tools, [1.0; TOOLS]);
    assert_eq!(more, 0.0);
}
