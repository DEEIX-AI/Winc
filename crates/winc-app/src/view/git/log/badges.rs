//! 提交的分支和标签徽章、作者头像。
//!
//! 徽章由提交的引用逐个生成（当前分支、本地分支、远程分支、标签），再按可用宽度排成一行；
//! 排不下的收成「+N」。头像按邮箱哈希取调色板里的颜色，同一个作者的颜色固定。

use super::*;

/// 一个提交的引用徽章：（文字，底色，字色）。
///
/// 当前分支铺强调色；和远程分支在同一个提交上（已经同步）的本地分支只画一个，后面写
/// 远程名；远程分支、标签各一种淡色。
pub(super) fn badges(p: &Palette, commit: &Commit) -> Vec<(String, Color, Color)> {
    let local: Vec<&str> = commit
        .refs
        .iter()
        .filter_map(|r| match r {
            Ref::Head(n) | Ref::Branch(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let synced = |name: &str| {
        commit.refs.iter().find_map(|r| match r {
            Ref::Remote(remote) => remote
                .split_once('/')
                .filter(|(_, branch)| *branch == name)
                .map(|(origin, _)| origin.to_owned()),
            _ => None,
        })
    };
    let soft = |c: Color| c.alpha(c.a * 0.16);
    let mut out = Vec::new();
    for r in &commit.refs {
        let (text, fill, ink) = match r {
            Ref::Detached => ("HEAD".to_owned(), p.accent_fill, p.text_on_accent),
            Ref::Head(name) | Ref::Branch(name) => {
                let text = match synced(name) {
                    Some(origin) => format!("{name} · {origin}"),
                    None => name.clone(),
                };
                if matches!(r, Ref::Head(_)) {
                    (text, p.accent_fill, p.text_on_accent)
                } else {
                    (text, soft(p.accent), p.accent)
                }
            }
            Ref::Remote(name) => {
                let branch = name.split_once('/').map_or(name.as_str(), |(_, b)| b);
                if local.contains(&branch) {
                    continue;
                }
                (name.clone(), soft(p.text_muted), p.text_muted)
            }
            Ref::Tag(name) => (name.clone(), soft(p.warning), p.warning),
        };
        out.push((text, fill, ink));
    }
    out
}

/// 把一行引用徽章排进宽度 `room`（逻辑像素），返回排好的（文字，底色，字色）。
///
/// 每个徽章最宽 [`BADGE_MAX`]，放不下的从末尾截短；余下的连截短到 [`BADGE_MIN`] 都放不下
/// 时，收成一个淡色的「+N」。第一个徽章总要排上。
pub(super) fn fit_badges(
    app: &App,
    frame: &mut Frame<'_>,
    badges: &[(String, Color, Color)],
    room: f32,
) -> Vec<(String, Color, Color)> {
    let p = app.palette;
    let size = app.fonts.ui_small;
    let more_w = |frame: &mut Frame<'_>, n: usize| {
        frame.measure(FontKind::Ui, size, &format!("+{n}")) + 12.0
    };
    let mut out = Vec::new();
    let mut used = 0.0;
    for (i, (text, fill, ink)) in badges.iter().enumerate() {
        let left = badges.len() - i - 1;
        // 后面还有徽章时给「+N」留出地方。
        let reserve = if left > 0 {
            more_w(frame, left) + BADGE_GAP
        } else {
            0.0
        };
        let fits = (room - used - reserve).min(BADGE_MAX);
        let full = frame.measure(FontKind::Ui, size, text) + 12.0;
        if i > 0 && full.min(BADGE_MAX) > fits && fits < BADGE_MIN {
            let soft = p.text_muted.alpha(p.text_muted.a * 0.16);
            out.push((format!("+{}", badges.len() - i), soft, p.text_muted));
            return out;
        }
        let fits = if i == 0 {
            BADGE_MAX.min(room).max(BADGE_MIN)
        } else {
            fits
        };
        let text = truncate_end(frame, text, size, (fits - 12.0).max(0.0)).into_owned();
        used += frame.measure(FontKind::Ui, size, &text) + 12.0 + BADGE_GAP;
        out.push((text, *fill, *ink));
    }
    out
}

/// [`fit_badges`] 排好的一行徽章的总宽（逻辑像素），不含最后一个后面的间隔。
pub(super) fn badges_width(
    app: &App,
    frame: &mut Frame<'_>,
    badges: &[(String, Color, Color)],
) -> f32 {
    let size = app.fonts.ui_small;
    let total: f32 = badges
        .iter()
        .map(|(text, ..)| frame.measure(FontKind::Ui, size, text) + 12.0 + BADGE_GAP)
        .sum();
    (total - BADGE_GAP).max(0.0)
}

/// 从 `x` 起画 [`fit_badges`] 排好的一行徽章。
pub(super) fn draw_badges(
    app: &App,
    frame: &mut Frame<'_>,
    (x, row): (f32, Rect),
    badges: &[(String, Color, Color)],
    clip: Rect,
) {
    let mut x = x;
    for (text, fill, ink) in badges {
        x = draw_badge(app, frame, (x, row), text, *fill, *ink, clip) + BADGE_GAP;
    }
}

/// 画一个圆角小徽章，高为行高减 6 逻辑像素、最多 18 逻辑像素（经验值）；返回右边缘的 x
/// 坐标。
pub(super) fn draw_badge(
    app: &App,
    frame: &mut Frame<'_>,
    (x, row): (f32, Rect),
    text: &str,
    fill: Color,
    ink: Color,
    clip: Rect,
) -> f32 {
    let w = frame.measure(FontKind::Ui, app.fonts.ui_small, text) + 12.0;
    let h = (row.h - 6.0).min(18.0);
    let rect = Rect::new(x, (row.y + (row.h - h) / 2.0).round(), w, h);
    frame.rect_clipped(rect, fill, 4.0, clip);
    ui_text_small(frame, app, x + 6.0, rect, text, ink, clip);
    rect.right()
}

/// 作者的小头像：一个按邮箱取色的圆，里面是名字的第一个字。
///
/// 邮箱按 djb2 哈希（初值 5381、乘 33 再异或）取调色板里的颜色，同一个作者的颜色固定。
pub(super) fn draw_avatar(
    app: &App,
    frame: &mut Frame<'_>,
    commit: &Commit,
    (cx, cy): (f32, f32),
    clip: Rect,
) {
    let p = app.palette;
    let hash = commit
        .email
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let (fill, ink) = p.badges[hash as usize % p.badges.len()];
    // 头像的半径（逻辑像素，经验值）。
    let r = 7.0;
    frame.rect_clipped(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), fill, r, clip);
    let initial: String = commit
        .author
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    let size = app.fonts.ui_small * 0.8;
    let w = frame.measure(FontKind::Ui, size, &initial);
    let rect = Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r);
    caps_label(frame, app, size, (cx - w / 2.0, rect), &initial, ink, clip);
}
