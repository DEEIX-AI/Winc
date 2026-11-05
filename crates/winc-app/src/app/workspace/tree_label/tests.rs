//! `tree_label` 的测试：用户操作引起的过渡、相同的前缀、其他变化不播放过渡，以及不再显示的行。

use super::*;

/// 把字符串列表转成 `sync` 的参数。
fn parts<'a>(names: &[&'a str]) -> Vec<Cow<'a, str>> {
    names.iter().map(|n| Cow::Borrowed(*n)).collect()
}

/// 返回级数为 `levels` 的状态；级数变化表示路径变了。
fn cause(levels: usize) -> LabelCause {
    LabelCause {
        hovered: false,
        levels,
        current: 0,
    }
}

/// 返回开始了一帧的状态。
fn labels(animate: bool) -> TreeLabels {
    let mut labels = TreeLabels::default();
    labels.begin_frame(animate);
    labels
}

#[test]
fn a_row_seen_for_the_first_time_does_not_animate() {
    let labels = labels(true);
    let view = labels.sync(Path::new("a"), cause(0), &parts(&["a"]), 10.0);
    assert_eq!((view.width, view.alpha), (10.0, 1.0));
    assert!(view.old.is_empty());
    assert!(!labels.moving());
}

#[test]
fn a_longer_path_keeps_the_common_prefix_and_grows() {
    let labels = labels(true);
    labels.sync(Path::new("a"), cause(0), &parts(&["a"]), 10.0);
    let view = labels.sync(Path::new("a"), cause(3), &parts(&["a", "b", "c"]), 100.0);
    assert!(labels.moving());
    assert_eq!(view.old, ["a"]);
    assert_eq!(view.common, 1);
    // 刚开始：宽度还在起点附近，旧内容清晰，新内容还没淡入。
    assert!(view.width < 50.0, "{}", view.width);
    assert!(view.old_alpha > 0.9, "{}", view.old_alpha);
    assert!(view.alpha < 0.1, "{}", view.alpha);
}

#[test]
fn a_width_change_alone_does_not_fade_the_text() {
    let labels = labels(true);
    labels.sync(Path::new("a"), cause(0), &parts(&["long"]), 100.0);
    let hovered = LabelCause {
        hovered: true,
        ..cause(0)
    };
    let view = labels.sync(Path::new("a"), hovered, &parts(&["long"]), 300.0);
    assert!(labels.moving());
    assert!(view.old.is_empty());
    assert_eq!(view.alpha, 1.0);
    assert!(view.width < 200.0);
    assert!(view.progress < 0.5, "{}", view.progress);
}

#[test]
fn without_animation_the_row_shows_its_final_state() {
    let labels = labels(false);
    labels.sync(Path::new("a"), cause(0), &parts(&["a"]), 10.0);
    let view = labels.sync(Path::new("a"), cause(2), &parts(&["a", "b"]), 100.0);
    assert_eq!(view.width, 100.0);
    assert!(view.old.is_empty());
    assert!(!labels.moving());
}

#[test]
fn rows_no_longer_drawn_are_forgotten() {
    let mut labels = labels(true);
    labels.sync(Path::new("a"), cause(0), &parts(&["a"]), 10.0);
    labels.begin_frame(true);
    labels.end_frame();
    assert_eq!(labels.peek(Path::new("a")), None);
    // 再次出现时不播放过渡。
    labels.begin_frame(true);
    let view = labels.sync(Path::new("a"), cause(2), &parts(&["a", "b"]), 100.0);
    assert_eq!(view.width, 100.0);
    assert!(!labels.moving());
}

#[test]
fn changes_without_a_user_cause_apply_at_once() {
    // 拖动侧栏宽度：可用空间变了，折叠的内容和宽度跟着变，不播放过渡。
    let labels = labels(true);
    labels.sync(Path::new("a"), cause(4), &parts(&["a", "…", "d"]), 100.0);
    let view = labels.sync(Path::new("a"), cause(4), &parts(&["a", "c", "d"]), 140.0);
    assert_eq!(view.width, 140.0);
    assert!(view.old.is_empty());
    assert!(!labels.moving());
}

#[test]
fn a_running_transition_continues_toward_a_new_width() {
    let labels = labels(true);
    labels.sync(Path::new("a"), cause(1), &parts(&["a"]), 10.0);
    labels.sync(Path::new("a"), cause(2), &parts(&["a", "b"]), 100.0);
    let view = labels.sync(Path::new("a"), cause(2), &parts(&["a", "b"]), 120.0);
    assert!(labels.moving());
    assert!(view.width < 60.0, "{}", view.width);
}
