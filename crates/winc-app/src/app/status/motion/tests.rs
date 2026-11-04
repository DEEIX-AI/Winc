//! `motion` 的测试：项的出现、消失、宽度变化时的文字淡入淡出，以及数字变化不启动过渡。

use super::*;

impl StatusMotion {
    /// 把进行中的过渡都停在开始后 `at` 处，使测试和截图的结果固定。
    pub(crate) fn freeze(&mut self, at: Duration) {
        let frozen = |_: Clock| Clock::frozen(at, STATUS_SLIDE);
        for row in &mut self.rows {
            row.width.clock = row.width.clock.map(frozen);
            if let Some(fading) = &mut row.previous {
                fading.clock = frozen(fading.clock);
            }
            row.presence = match row.presence {
                Presence::Shown => Presence::Shown,
                Presence::Entering(c) => Presence::Entering(frozen(c)),
                Presence::Leaving(c) => Presence::Leaving(frozen(c)),
            };
        }
    }
}

const DOCUMENT: &[StatusKey] = &[StatusKey::Position, StatusKey::Encoding];

fn entry(key: StatusKey, text: &str, width: f32) -> StatusEntry {
    StatusEntry {
        key,
        text: text.to_owned(),
        item: (key == StatusKey::Encoding).then_some(StatusItem::Encoding),
        content: width,
        width,
    }
}

/// 一组已经完全显示的项：位置 20 逻辑像素宽，编码 40 逻辑像素宽。
fn shown() -> StatusMotion {
    let mut motion = StatusMotion::default();
    let entries = vec![
        entry(StatusKey::Position, "1:1", 20.0),
        entry(StatusKey::Encoding, "UTF-8", 40.0),
    ];
    motion.sync(DOCUMENT, entries, false);
    motion
}

#[test]
fn items_grow_and_fade_in_when_they_appear() {
    let mut motion = StatusMotion::default();
    let rows = motion.sync(
        DOCUMENT,
        vec![entry(StatusKey::Encoding, "UTF-8", 40.0)],
        true,
    );
    assert!(rows[0].width < 1.0 && rows[0].alpha < 0.1, "{rows:?}");
    // 出现途中不可点击。
    assert_eq!(rows[0].item, None);

    // 过渡进行到一半时，宽度按缓出曲线已超过终值的一半。
    motion.freeze(STATUS_SLIDE / 2);
    let entries = vec![entry(StatusKey::Encoding, "UTF-8", 40.0)];
    let rows = motion.sync(DOCUMENT, entries.clone(), true);
    assert!(rows[0].width > 20.0 && rows[0].width < 40.0, "{rows:?}");
    assert!(rows[0].share > 0.5 && rows[0].share < 1.0);

    motion.freeze(STATUS_SLIDE);
    motion.settle();
    let rows = motion.sync(DOCUMENT, entries, true);
    assert_eq!((rows[0].width, rows[0].alpha), (40.0, 1.0));
    assert_eq!(rows[0].item, Some(StatusItem::Encoding));
    assert!(!motion.moving());
}

#[test]
fn items_shrink_and_fade_out_when_they_go_away() {
    let mut motion = shown();
    let rows = motion.sync(DOCUMENT, Vec::new(), true);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.item.is_none()));

    motion.freeze(STATUS_SLIDE);
    motion.settle();
    assert!(motion.sync(DOCUMENT, Vec::new(), true).is_empty());
    assert!(!motion.moving());
}

#[test]
fn a_wider_text_crossfades_while_the_width_eases() {
    let mut motion = shown();
    let entries = vec![
        entry(StatusKey::Position, "26:1（300 个字符）", 90.0),
        entry(StatusKey::Encoding, "UTF-8", 40.0),
    ];
    let rows = motion.sync(DOCUMENT, entries.clone(), true);
    let position = &rows[0];
    // 过渡开始时宽度仍为旧值，旧文字不透明，新文字透明。
    assert!((position.width - 20.0).abs() < 1.0);
    let (old, _, old_alpha) = position.previous.clone().unwrap();
    assert_eq!(old, "1:1");
    assert!(old_alpha > 0.9 && position.alpha < 0.1);

    motion.freeze(STATUS_SLIDE);
    motion.settle();
    let rows = motion.sync(DOCUMENT, entries, true);
    assert_eq!(rows[0].width, 90.0);
    assert!(rows[0].previous.is_none());
}

#[test]
fn a_new_number_of_the_same_width_replaces_the_text_at_once() {
    let mut motion = shown();
    let entries = vec![
        entry(StatusKey::Position, "2:1", 20.0),
        entry(StatusKey::Encoding, "UTF-8", 40.0),
    ];
    let rows = motion.sync(DOCUMENT, entries, true);
    assert_eq!(rows[0].text.0, "2:1");
    assert!(rows[0].previous.is_none());
    assert!(!motion.moving());
}

#[test]
fn without_animation_every_item_shows_its_final_state() {
    let mut motion = shown();
    let rows = motion.sync(
        DOCUMENT,
        vec![entry(StatusKey::Position, "3:1", 50.0)],
        false,
    );
    // 编码立即移除，位置立即换成新宽度。
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].width, rows[0].alpha), (50.0, 1.0));
    assert!(!motion.moving());
}

#[test]
fn groups_keep_their_own_items_in_declared_order() {
    let mut motion = shown();
    let update = [StatusKey::Update];
    let rows = motion.sync(
        &update,
        vec![entry(StatusKey::Update, "新版本", 30.0)],
        false,
    );
    assert_eq!(rows.len(), 1);
    // 另一组的同步不影响文档组的项，文档组仍按声明顺序排列。
    let keys: Vec<StatusKey> = motion
        .sync(
            DOCUMENT,
            vec![
                entry(StatusKey::Encoding, "UTF-8", 40.0),
                entry(StatusKey::Position, "1:1", 20.0),
            ],
            false,
        )
        .iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(keys, [StatusKey::Position, StatusKey::Encoding]);
}

#[test]
fn a_change_during_a_crossfade_keeps_fading_the_clearer_text() {
    let mut motion = shown();
    let wider = |text: &str, width: f32| {
        vec![
            entry(StatusKey::Position, text, width),
            entry(StatusKey::Encoding, "UTF-8", 40.0),
        ]
    };
    motion.sync(DOCUMENT, wider("1:1 · 正在同步…", 90.0), true);
    // 刚开始过渡时旧文字仍清晰：再次换文字时继续淡出原来的旧文字，不换成还看不见的那段。
    motion.freeze(STATUS_SLIDE / 20);
    let rows = motion.sync(DOCUMENT, wider("1:1 ↑1 · 正在同步…", 110.0), true);
    let (old, _, alpha) = rows[0].previous.clone().unwrap();
    assert_eq!(old, "1:1");
    assert!(alpha > 0.5, "{alpha}");
    assert_eq!(rows[0].text.0, "1:1 ↑1 · 正在同步…");

    // 新文字已比旧文字清晰时再换：淡出的是新文字，从它此刻的不透明度开始，不先跳回不透明。
    motion.freeze(STATUS_SLIDE * 3 / 4);
    let before = motion.sync(DOCUMENT, wider("1:1 ↑1 · 正在同步…", 110.0), true)[0].alpha;
    let rows = motion.sync(DOCUMENT, wider("1:1", 20.0), true);
    let (old, _, alpha) = rows[0].previous.clone().unwrap();
    assert_eq!(old, "1:1 ↑1 · 正在同步…");
    assert!((alpha - before).abs() < 0.05, "{alpha} vs {before}");
}
