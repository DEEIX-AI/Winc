//! 等宽格子中需要整形的文字：一段文字整形一次，按渲染层给出的字形簇逐个绘制。
//!
//! 每个字形簇（见 [`cpui_render::TextCluster`]）画在簇内字符所占的显示列中，光标和选区按显示列
//! 定位，因此总落在两个字形簇之间。编辑区和终端共用；两者计算显示列的方式不同，由调用方给出。

use cpui_render::{Color, Direction, Font, Frame, Rect};
use winc_core::text::shaping::needs_shaping;

/// 一次整形的字符数上限（字符）：更长的一段分几次整形，每次接在上一次最后一个完整的字形簇之后。
///
/// 64 大于常见字形簇的长度（字母与其符号、印度文字的连写辅音），也是按字素移动光标时的查找上限。
pub(in crate::view) const SHAPED_RUN: usize = 64;

/// 是否与前面的字一起整形：需要整形的字符，以及印度文字和阿拉伯文中控制连写的零宽（不）连接符。
pub(in crate::view) fn joins(c: char) -> bool {
    needs_shaping(c) || matches!(c, '\u{200c}' | '\u{200d}')
}

/// 整形绘制 `run`（至多 [`SHAPED_RUN`] 个字符），返回绘制的字节数；无法整形时返回 0。
///
/// 每个字形簇占其字符的显示列之和（`cells_of` 给出单个字符的列数）：较窄时居中，较宽时按比例
/// 缩小字号。`whole` 为假时只画第一个簇；`more` 为真表示同一段文字在 `run` 之后还有，最后一个簇
/// 可能不完整，留到下一次。
#[allow(clippy::too_many_arguments)]
pub(in crate::view) fn draw_shaped_run(
    frame: &mut Frame<'_>,
    (font, cell): (Font, f32),
    (x, baseline): (f32, f32),
    run: &str,
    cells_of: impl Fn(char) -> usize,
    (whole, more): (bool, bool),
    color: Color,
    clip: Rect,
) -> usize {
    let mut clusters = [(0, 0, 0.0); SHAPED_RUN];
    let mut count = 0;
    frame.clusters(font, run, Direction::Ltr, |cluster| {
        if count < SHAPED_RUN {
            clusters[count] = (cluster.range.start, cluster.range.end, cluster.width);
            count += 1;
        }
    });
    let clusters = &mut clusters[..count];
    clusters.sort_unstable_by_key(|&(start, ..)| start);
    let mut chars = run.char_indices().peekable();
    let (mut column, mut drawn) = (0, 0);
    for (n, &(start, end, width)) in clusters.iter().enumerate() {
        if n > 0 && (!whole || (more && end == run.len())) {
            break;
        }
        let mut span = 0;
        while let Some(&(at, c)) = chars.peek().filter(|&&(at, _)| at < end) {
            if at < start {
                column += cells_of(c);
            } else {
                span += cells_of(c);
            }
            chars.next();
        }
        // 只有不占列的字符（孤立的组合符号）时仍画在一列中。
        let cells = span.max(1);
        let room = cells as f32 * cell;
        let mut font = font;
        if width > room {
            font.size *= room / width;
        }
        let at = (x + column as f32 * cell, baseline);
        frame.cell_text(font, at, cell, cells, &run[start..end], color, clip);
        column += span;
        drawn = end;
    }
    drawn
}
