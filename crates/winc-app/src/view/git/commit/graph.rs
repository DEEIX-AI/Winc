//! 提交视图底部的提交图：标题行和最近若干提交，每行是车道、提交说明和作者。
//!
//! 与 Git 日志标签画的是同一张图（车道算法见 [`winc_core::graph`]，线条画法见
//! [`crate::view::git::log::graph`]）：最左边几列画车道，说明从车道右边开始，作者淡色排在行尾。
//! 一行一个提交，点一行打开该提交（见 [`App::press_graph_row`]）；行数多于可见高度时用滚轮滚动。

use winc_editor::git::Ref;

use super::*;
use crate::app::git::graph::{GRAPH_HEADER, GRAPH_ROW};
use crate::view::git::log::graph::lane_edge;
use crate::view::git::log::{GRAPH_FADED, GRAPH_LINE, LANE, MAX_LANES, lane_color};

/// 提交图一行的作者名字最多占的宽度（逻辑像素）：说明优先，作者只作提示。
const AUTHOR_MAX: f32 = 96.0;
/// 说明与作者之间的间隔（逻辑像素）。
const AUTHOR_GAP: f32 = 8.0;
/// 提交图左边到第一条车道的距离（逻辑像素）。
const GRAPH_PAD: f32 = 10.0;

/// 绘制提交图：上沿一条分隔线，一行标题，下面每行一个提交。
pub(super) fn draw_graph(
    app: &App,
    frame: &mut Frame<'_>,
    parts: &CommitParts,
    hits: &mut Vec<(Rect, CommitTarget)>,
) {
    let Some(area) = parts.graph else {
        return;
    };
    let p = app.palette;
    let clip = app.layout.sidebar.intersect(area);
    frame.rect_clipped(
        Rect::new(area.x, area.y, area.w, 1.0),
        p.separator,
        0.0,
        clip,
    );
    let size = app.fonts.ui_small;
    let head = Rect::new(area.x, area.y, area.w, GRAPH_HEADER);
    label(
        frame,
        app,
        size,
        (area.x + 8.0, head),
        t!("git-graph-title"),
        p.text_muted,
        clip,
    );
    let rows_area = Rect::new(
        area.x,
        head.bottom(),
        area.w,
        (area.h - GRAPH_HEADER).max(0.0),
    );
    let graph = &app.git.graph;
    // 读取出错时在标题行右端说明，图里留着上一次的内容。
    if let Some(error) = &graph.error {
        let text = truncate_end(frame, error, size, (rows_area.w - 16.0).max(0.0));
        let w = frame.measure(FontKind::Ui, size, &text);
        label(
            frame,
            app,
            size,
            (head.right() - 8.0 - w, head),
            &text,
            p.text_dim,
            clip,
        );
    }
    if graph.is_empty() {
        return;
    }
    // 车道最多画 `MAX_LANES` 条，超出的挤在最右边。
    let lanes = graph
        .rows
        .iter()
        .map(|row| row.width)
        .max()
        .unwrap_or(1)
        .min(MAX_LANES);
    let x0 = area.x + GRAPH_PAD;
    let text_x = x0 + lanes as f32 * LANE + 6.0;
    let scroll = app.git.graph.scroll;
    let shown = rows_area.h;
    for (i, (commit, row)) in graph.commits.iter().zip(&graph.rows).enumerate() {
        let top = rows_area.y + i as f32 * GRAPH_ROW - scroll;
        if top + GRAPH_ROW <= rows_area.y || top >= rows_area.y + shown {
            continue;
        }
        let row_rect = Rect::new(area.x, top, area.w, GRAPH_ROW).intersect(rows_area);
        let band = (app.pointer.hover == Hover::Commit(CommitTarget::GraphRow(i)))
            .then(|| Rect::new(area.x + 6.0, top + 1.0, area.w - 12.0, GRAPH_ROW - 2.0));
        if let Some(band) = band {
            frame.rect_clipped(band.intersect(rows_area), p.hover, RADIUS_ROW, clip);
        }
        let mid = top + GRAPH_ROW / 2.0;
        let ink = |color: u8| lane_color(p, color);
        // 上半行、下半行的线。淡色与行底色混好再画，圆头不会在行的交界处叠出深色。
        let bg = if band.is_some() {
            crate::view::workspace::panel::over(p.editor_bg, p.hover)
        } else {
            p.editor_bg
        };
        for (segments, above) in [(&row.up, true), (&row.down, false)] {
            for s in segments {
                let color = ink(s.color);
                let color = if s.line == row.line {
                    color
                } else {
                    crate::view::workspace::panel::over(bg, color.alpha(color.a * GRAPH_FADED))
                };
                let (top_y, bottom_y) = if above {
                    (top, mid)
                } else {
                    (mid, top + GRAPH_ROW)
                };
                let from = (
                    x0 + s.from.min(MAX_LANES - 1) as f32 * LANE + LANE / 2.0,
                    top_y,
                );
                let to = (
                    x0 + s.to.min(MAX_LANES - 1) as f32 * LANE + LANE / 2.0,
                    bottom_y,
                );
                let edge = lane_edge(from, to, above);
                frame.stroke(&edge, GRAPH_LINE, color, clip);
            }
        }
        // 圆点：`HEAD` 是空心圆，合并提交小一圈。
        let cx = x0 + row.lane.min(MAX_LANES - 1) as f32 * LANE + LANE / 2.0;
        let head_commit = commit
            .refs
            .iter()
            .any(|r| matches!(r, Ref::Head(_) | Ref::Detached));
        let r = if head_commit {
            4.0
        } else if commit.parents.len() > 1 {
            2.75
        } else {
            3.25
        };
        let dot = Rect::new(cx - r, mid - r, 2.0 * r, 2.0 * r);
        frame.rect_clipped(dot, ink(row.color), r, clip);
        if head_commit {
            let hole = r - GRAPH_LINE;
            frame.rect_clipped(
                Rect::new(cx - hole, mid - hole, 2.0 * hole, 2.0 * hole),
                if band.is_some() { p.hover } else { p.editor_bg },
                hole,
                clip,
            );
        }
        // 说明从车道右边开始，作者淡色排在行尾；放不下时先截说明。
        let author_w = frame
            .measure(FontKind::Ui, size, &commit.author)
            .min(AUTHOR_MAX);
        let author_x = (row_rect.right() - 8.0 - author_w).max(text_x + AUTHOR_GAP);
        let subject_room = (author_x - AUTHOR_GAP - text_x).max(0.0);
        let subject = truncate_end(frame, &commit.subject, size, subject_room);
        label(frame, app, size, (text_x, row_rect), &subject, p.text, clip);
        let author = truncate_end(frame, &commit.author, size, author_w);
        label(
            frame,
            app,
            size,
            (author_x, row_rect),
            &author,
            p.text_dim,
            clip,
        );
        hits.push((row_rect, CommitTarget::GraphRow(i)));
    }
}
