//! 日志标签绘制的测试：提交图车道连线的拐角形状、文字折行与末尾截短、说明中筛选文字的匹配。
//!
//! 坐标都是逻辑像素，比较时容差取 0.01。

use super::*;

fn ends(edge: &[Stroke]) -> Vec<((f32, f32), (f32, f32))> {
    edge.iter()
        .map(|s| match *s {
            Stroke::Line(x0, y0, x1, y1) => ((x0, y0), (x1, y1)),
            _ => panic!("提交图的线只有直线段"),
        })
        .collect()
}

fn near(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
}

#[test]
fn lane_edges_run_level_with_the_dot_then_turn_a_rounded_corner() {
    for into_dot in [false, true] {
        for (a, b) in [((10.0, 0.0), (58.0, 12.0)), ((58.0, 0.0), (10.0, 12.0))] {
            let edge = ends(&lane_edge(a, b, into_dot));
            // 一笔连下来：从 a 到 b，每段接在上一段之后。
            assert!(near(edge[0].0, a));
            assert!(near(edge.last().unwrap().1, b));
            assert!(edge.windows(2).all(|w| near(w[0].1, w[1].0)));
            // 圆点那一头是横着的（和圆点同高），另一头是竖着的。
            let (level, upright) = if into_dot {
                (*edge.last().unwrap(), edge[0])
            } else {
                (edge[0], *edge.last().unwrap())
            };
            assert!((level.0.1 - level.1.1).abs() < 0.01);
            assert!((upright.0.0 - upright.1.0).abs() < 0.01);
            // 拐角不超出两条车道之间。
            let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
            assert!(
                edge.iter()
                    .all(|(p, q)| (lo - 0.01..=hi + 0.01).contains(&p.0)
                        && (lo - 0.01..=hi + 0.01).contains(&q.0))
            );
        }
    }
    // 同一条车道是一条竖线。
    assert_eq!(lane_edge((5.0, 0.0), (5.0, 12.0), false).len(), 1);
}

/// 折行在词与词之间换行，换行处不留空格，比一行还长的词按字拆开；截短时「…」前不留空白。
#[test]
fn wrap_and_truncate_text() {
    let mut renderer =
        winc_render::Renderer::offscreen(64.0, 64.0, 1.0, Color::hex(0), None, None).unwrap();
    renderer.draw(|frame| {
        let size = 13.0;
        let mut w = |s: &str| frame.measure(FontKind::Ui, size, s);
        let word = w("hello").max(w("world")) + 0.01;
        let (abcd, ab, ell) = (w("abcd"), w("ab "), w("…"));

        assert_eq!(wrap(frame, size, "hello world", word), ["hello", "world"]);
        assert_eq!(wrap(frame, size, "hello world", 1000.0), ["hello world"]);
        assert_eq!(wrap(frame, size, "abcdefgh", abcd + 0.01), ["abcd", "efgh"]);
        assert_eq!(wrap(frame, size, "你好世界", 1000.0), ["你好世界"]);
        assert_eq!(wrap(frame, size, "", 10.0), [""]);

        let fits = truncate_end(frame, "ab cdef", size, 1000.0);
        assert!(matches!(fits, Cow::Borrowed("ab cdef")));
        assert_eq!(truncate_end(frame, "ab cdef", size, ab + ell + 0.01), "ab…");
    });
}

#[test]
fn filter_matches_ignore_case_and_do_not_overlap() {
    assert_eq!(matches("Fix fix FIX", "fix"), [0..3, 4..7, 8..11]);
    assert_eq!(matches("aaaa", "aa"), [0..2, 2..4]);
    assert_eq!(matches("修复登录：登录失败", "登录"), [6..12, 15..21]);
    assert!(matches("anything", "").is_empty());
    assert!(matches("short", "shorter").is_empty());
}
