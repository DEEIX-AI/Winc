//! mermaid 图的布局和画法（解析在 `winc_preview::markdown::mermaid`）。
//!
//! 配色取 mermaid 默认主题的色值，深色主题另用一组暗色。流程图按分层排：去掉环，按最长路径分层，
//! 跨层的连线中间插虚节点，逐层按邻居的平均位置排顺序减少交叉，再按邻居对齐定横坐标；连线上的文字
//! 也放在虚节点里。时序图、饼图见子模块。坐标都是逻辑像素。

use winc_preview::markdown::mermaid::{self, Diagram, Dir, Edge, Flow, Head, Line, Shape};
use winc_preview::markdown::{Align, Inline, Run, Style};
use winc_render::{Color, Rect, Stroke};

use super::math::polyline;
use super::memo::Piece;
use super::{Cx, Env, Flow as Laid, Item, Para};
use crate::icons::rounded_outline;

mod pie;
mod rank;
mod sequence;

use rank::layout;

/// 图的配色。
#[derive(Clone, Copy)]
pub(super) struct Look {
    /// 节点和连线上的文字颜色。
    text: Color,
    /// 节点的填充色。
    fill: Color,
    /// 节点的边框色。
    border: Color,
    /// 连线和箭头的颜色。
    line: Color,
    /// 连线文字背后的底色（半透明）。
    label_bg: Color,
    /// 子图框的填充色。
    group_fill: Color,
    /// 子图框的边框色。
    group_border: Color,
    /// 时序图注释框的填充色。
    note_fill: Color,
    /// 时序图注释框的边框色。
    note_border: Color,
    /// 是否配深色主题。
    dark: bool,
}

impl Look {
    /// 按深浅主题取一组颜色。
    ///
    /// 色值取自 mermaid 默认主题，深色主题另用一组暗色（节点、边框、连线各一组）。
    fn new(dark: bool, text: Color) -> Look {
        if dark {
            Look {
                text,
                fill: Color::hex(0x1f2020),
                border: Color::hex(0xcccccc),
                line: Color::hex(0xd3d3d3),
                label_bg: Color::hex(0x585858),
                group_fill: Color::hex(0x474949),
                group_border: Color::hex(0xffffff).alpha(0.25),
                note_fill: Color::hex(0x585858),
                note_border: Color::hex(0xcccccc),
                dark,
            }
        } else {
            Look {
                text,
                fill: Color::hex(0xececff),
                border: Color::hex(0x9370db),
                line: Color::hex(0x333333),
                label_bg: Color::hex(0xe8e8e8),
                group_fill: Color::hex(0xffffde),
                group_border: Color::hex(0xaaaa33),
                note_fill: Color::hex(0xfff5ad),
                note_border: Color::hex(0xaaaa33),
                dark,
            }
        }
    }
}

/// 排好的一段图里的文字：单独排好的段和它的宽高（逻辑像素）。
pub(super) struct Label {
    /// 单独排好的这一段（坐标从 0 起）。
    piece: Piece,
    /// 段的宽度（逻辑像素）。
    w: f32,
    /// 段的高度（逻辑像素）。
    h: f32,
}

impl Cx<'_, '_> {
    /// 排一张 mermaid 图，左上角在 `(x, y)`、最宽 `width`（逻辑像素），水平居中。
    ///
    /// 认不出、不会画的图返回 `None`（按代码块显示）。
    pub(super) fn diagram(
        &mut self,
        source: &str,
        x: f32,
        y: f32,
        width: f32,
        env: Env,
        out: &mut Vec<Item>,
    ) -> Option<Laid> {
        let diagram = mermaid::parse(source)?;
        if matches!(diagram, Diagram::Unsupported(_)) {
            return None;
        }
        let look = Look::new(self.dark, env.color);
        let (text_len, chars) = (self.text.len(), self.chars);
        let full = self.base * 0.875;
        let mut size = full;
        let mut items = Vec::new();
        // 图宽于一行时缩小字号重排，直到放得下，使整张图无需横向滚动即可完整显示。文字折行
        // 不随字号严格按比例变化，因此需要多次尝试。宽度按实际绘制范围计算：排版后的图左侧
        // 可能留有空白（子图、连线避让处），因此将图平移至左对齐。
        let mut bounds = (0.0, 0.0);
        let mut h = 0.0;
        // 最多缩小字号重排 5 次（经验值）：再缩小也看不清，剩下的宽度直接超出。
        for _ in 0..5 {
            items.clear();
            self.text.truncate(text_len);
            self.chars = chars;
            let laid = match &diagram {
                Diagram::Flow(flow) => self.flowchart(flow, size, look, &mut items),
                Diagram::Sequence(seq) => self.sequence(seq, size, look, &mut items),
                Diagram::Pie(pie) => self.pie(pie, size, look, &mut items),
                Diagram::Unsupported(_) => return None,
            };
            h = laid.1;
            bounds = items
                .iter()
                .map(Item::span_x)
                .fold((f32::MAX, f32::MIN), |(a, b), (l, r)| (a.min(l), b.max(r)));
            if bounds.0 > bounds.1 {
                bounds = (0.0, laid.0);
            }
            let w = bounds.1 - bounds.0;
            if w <= width {
                break;
            }
            let next = size * width / w * 0.98;
            if next >= size {
                break;
            }
            size = next;
        }
        let w = bounds.1 - bounds.0;
        let dx = x + ((width - w) / 2.0).max(0.0) - bounds.0;
        out.extend(items.iter().map(|item| item.moved(dx, y, 0)));
        self.end_paragraph();
        Some(Laid {
            height: h,
            first_baseline: None,
        })
    }

    /// 单独排一段图里的文字（`\n` 分行），字号 `size`，最宽 `max`，各行居中。
    fn label(&mut self, text: &str, size: f32, color: Color, max: f32) -> Label {
        let para = Para {
            size,
            bold: false,
            color,
            line: 1.35,
            align: Align::Center,
        };
        let inline = Inline {
            text: text.to_owned(),
            runs: vec![Run {
                end: text.len(),
                style: Style::default(),
            }],
            ..Inline::default()
        };
        if text.is_empty() {
            let piece = self.isolated(|_, _| Laid {
                height: 0.0,
                first_baseline: None,
            });
            return Label {
                piece,
                w: 0.0,
                h: 0.0,
            };
        }
        let (word, line) = self.measure(&inline, para);
        let w = line.min(max).max(word.min(max)).ceil();
        let piece = self.isolated(|cx, out| cx.flow_at(&inline, 0.0, 0.0, w, para, out));
        let h = piece.height;
        Label { piece, w, h }
    }

    /// 把排好的文字放在中心 `(cx, cy)`。
    fn put_label(&mut self, label: &Label, cx: f32, cy: f32, out: &mut Vec<Item>) {
        self.paste(&label.piece, cx - label.w / 2.0, cy - label.h / 2.0, 0, out);
    }

    /// 排流程图，返回宽高。
    fn flowchart(&mut self, f: &Flow, s: f32, look: Look, out: &mut Vec<Item>) -> (f32, f32) {
        let vertical = matches!(f.dir, Dir::Down | Dir::Up);
        let labels: Vec<Label> = f
            .nodes
            .iter()
            .map(|n| self.label(&n.label, s, look.text, s * 14.0))
            .collect();
        let sizes: Vec<(f32, f32)> = f
            .nodes
            .iter()
            .zip(&labels)
            .map(|(n, l)| shape_size(n.shape, l.w, l.h, s, vertical))
            .collect();
        let edge_labels: Vec<Label> = f
            .edges
            .iter()
            .map(|e| self.label(&e.label, s * 0.9, look.text, s * 10.0))
            .collect();
        let titles: Vec<Label> = f
            .groups
            .iter()
            .map(|g| self.label(&g.title, s * 0.9, look.text, s * 16.0))
            .collect();
        let heights: Vec<f32> = titles.iter().map(|t| t.h).collect();
        let plan = layout(f, &sizes, &edge_labels, &heights, s, f.dir);
        let place = |p: (f32, f32)| plan.place(p, f.dir);
        let (w, h) = if vertical {
            (plan.cross, plan.along)
        } else {
            (plan.along, plan.cross)
        };
        let rect_of = |i: usize| {
            let (cx, cy) = place(plan.centers[i]);
            let (nw, nh) = sizes[i];
            Rect::new(cx - nw / 2.0, cy - nh / 2.0, nw, nh)
        };

        // 子图的框，外面的先画；标题等连线画完再写，不被线压住。
        let boxes: Vec<Option<Rect>> = plan
            .boxes
            .iter()
            .map(|b| {
                (b[0] <= b[2]).then(|| {
                    let (x0, y0) = place((b[0], b[1]));
                    let (x1, y1) = place((b[2], b[3]));
                    Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
                })
            })
            .collect();
        let mut outer_first: Vec<usize> = (0..boxes.len()).collect();
        outer_first.sort_by(|&a, &b| {
            let area = |g: usize| boxes[g].map_or(0.0, |r| r.w * r.h);
            area(b).total_cmp(&area(a))
        });
        for &g in &outer_first {
            let Some(rect) = boxes[g] else { continue };
            out.push(Item::Fill {
                rect,
                color: look.group_fill,
                radius: 0.0,
            });
            out.push(Item::path(outline(rect, 0.0), 1.0, look.group_border));
        }

        // 连线。
        for (k, edge) in f.edges.iter().enumerate() {
            let Some(route) = plan.routes[k].as_ref() else {
                continue;
            };
            let mut points: Vec<(f32, f32)> = route.points.iter().map(|&p| place(p)).collect();
            if edge.from == edge.to {
                points = self_loop(rect_of(edge.from), s);
            } else if points.len() >= 2 {
                let n = points.len();
                points[0] = clip(f.nodes[edge.from].shape, rect_of(edge.from), points[1]);
                points[n - 1] = clip(f.nodes[edge.to].shape, rect_of(edge.to), points[n - 2]);
            }
            let points = smooth(&points);
            self.edge(edge, &points, s, look, out);
            if !edge.label.is_empty() {
                let label = &edge_labels[k];
                let (cx, cy) = match route.label {
                    Some(p) => place(p),
                    None => points[points.len() / 2],
                };
                let pad = s * 0.15;
                out.push(Item::Fill {
                    rect: Rect::new(
                        cx - label.w / 2.0 - pad,
                        cy - label.h / 2.0 - pad,
                        label.w + pad * 2.0,
                        label.h + pad * 2.0,
                    ),
                    color: look.label_bg.alpha(0.85),
                    radius: 2.0,
                });
                self.put_label(label, cx, cy, out);
            }
        }

        for &g in &outer_first {
            let (Some(rect), title) = (boxes[g], &titles[g]) else {
                continue;
            };
            if title.w > 0.0 {
                let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + s * 0.2 + title.h / 2.0);
                out.push(Item::Fill {
                    rect: Rect::new(
                        cx - title.w / 2.0 - s * 0.2,
                        cy - title.h / 2.0,
                        title.w + s * 0.4,
                        title.h,
                    ),
                    color: look.group_fill,
                    radius: 2.0,
                });
                self.put_label(title, cx, cy, out);
            }
        }

        // 节点画在连线上面。
        for (i, node) in f.nodes.iter().enumerate() {
            let r = rect_of(i);
            draw_shape(node.shape, r, look, out);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let dy = if node.shape == Shape::Cylinder {
                cylinder_ry(r.w) / 2.0
            } else {
                0.0
            };
            self.put_label(&labels[i], cx, cy + dy, out);
        }
        bounds(out, w, h)
    }

    /// 画一条连线 `points`（已经平滑过）和两头的线头。
    fn edge(
        &mut self,
        edge: &Edge,
        points: &[(f32, f32)],
        s: f32,
        look: Look,
        out: &mut Vec<Item>,
    ) {
        if edge.line == Line::Hidden || points.len() < 2 {
            return;
        }
        let width = match edge.line {
            Line::Thick => s * 0.25,
            _ => s * 0.1,
        };
        let mut points = points.to_vec();
        let len = s * 0.6;
        for (head, at_end) in [(edge.head, true), (edge.tail, false)] {
            if head == Head::None {
                continue;
            }
            let n = points.len();
            let (tip, from) = if at_end {
                (points[n - 1], points[n - 2])
            } else {
                (points[0], points[1])
            };
            let base = arrow_head(head, tip, from, len, look.line, out);
            if at_end {
                points[n - 1] = base;
            } else {
                points[0] = base;
            }
        }
        let strokes = if edge.line == Line::Dotted {
            dashed(&points, s * 0.25, s * 0.25)
        } else {
            polyline(&points)
        };
        if !strokes.is_empty() {
            out.push(Item::path(strokes, width, look.line));
        }
    }
}

/// 所有项的外框（不小于 `(w, h)`），左上角挪到原点，返回宽高（逻辑像素）。
///
/// 外框四周各留 2 逻辑像素，避免线宽被裁掉一半。
fn bounds(items: &mut [Item], w: f32, h: f32) -> (f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (0f32, 0f32, w, h);
    for item in items.iter() {
        let r = match item {
            Item::Fill { rect, .. }
            | Item::Path { rect, .. }
            | Item::Polygon { rect, .. }
            | Item::Link { rect, .. } => *rect,
            Item::Text {
                x,
                baseline,
                size,
                span,
                ..
            } => {
                let w = span.as_ref().map_or(0.0, |s| s.advances.iter().sum());
                Rect::new(*x, baseline - size, w, size * 1.3)
            }
            _ => continue,
        };
        (x0, y0, x1, y1) = (
            x0.min(r.x),
            y0.min(r.y),
            x1.max(r.right()),
            y1.max(r.bottom()),
        );
    }
    let pad = 2.0;
    for item in items.iter_mut() {
        *item = item.moved(pad - x0, pad - y0, 0);
    }
    (x1 - x0 + pad * 2.0, y1 - y0 + pad * 2.0)
}

/// 返回装下 `tw × th` 的文字（逻辑像素）所需的形状大小（宽、高）。
///
/// `vertical` 是上下走向的图（横条横着放）。
fn shape_size(shape: Shape, tw: f32, th: f32, s: f32, vertical: bool) -> (f32, f32) {
    let (px, py) = (s * 1.0, s * 0.6);
    let (w, h) = (tw + px * 2.0, th + py * 2.0);
    match shape {
        Shape::Rect | Shape::Round => (w, h),
        Shape::Stadium => (w + h / 2.0, h),
        Shape::Subroutine => (w + s, h),
        Shape::Asym => (w + h / 4.0, h),
        Shape::Cylinder => (w, h + cylinder_ry(w) * 2.0),
        Shape::Circle => {
            let d = tw.max(th) + px * 1.2;
            (d, d)
        }
        Shape::DoubleCircle => {
            let d = tw.max(th) + px * 1.2 + s * 0.6;
            (d, d)
        }
        Shape::Diamond => {
            let side = (tw + th) * 0.8 + px * 2.0;
            (side.max(w * 1.3), (side * 0.75).max(h * 1.5))
        }
        Shape::Hexagon => (w + h / 2.0, h),
        Shape::Lean | Shape::LeanLeft | Shape::Trapezoid | Shape::TrapezoidDown => (w + h * 0.6, h),
        Shape::Start | Shape::End => (s, s),
        Shape::Bar if vertical => (s * 5.0, s * 0.5),
        Shape::Bar => (s * 0.5, s * 5.0),
    }
}

/// 圆柱上下椭圆的半高（逻辑像素）：按宽度的 6% 取，限制在 4–10 之间，窄节点不会太扁、
/// 宽节点不会太鼓。
fn cylinder_ry(w: f32) -> f32 {
    (w * 0.06).clamp(4.0, 10.0)
}

/// 矩形 `r` 的四条边（`radius` 为圆角）。
fn outline(r: Rect, radius: f32) -> Vec<Stroke> {
    if radius > 0.0 {
        rounded_outline(r.x, r.y, r.w, r.h, radius).to_vec()
    } else {
        closed(&[
            (r.x, r.y),
            (r.right(), r.y),
            (r.right(), r.bottom()),
            (r.x, r.bottom()),
        ])
    }
}

/// 首尾相连的折线。
fn closed(points: &[(f32, f32)]) -> Vec<Stroke> {
    let mut all = points.to_vec();
    all.extend(points.first());
    polyline(&all)
}

/// 中心 `(cx, cy)`、半轴 `rx`、`ry` 的椭圆上从角度 `from` 到 `to` 的点。
fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|k| {
            let a = from + (to - from) * k as f32 / n as f32;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// 画节点的形状（填色和边框）。
///
/// 边框粗 1.2 逻辑像素。
fn draw_shape(shape: Shape, r: Rect, look: Look, out: &mut Vec<Item>) {
    let (fill, border) = (look.fill, look.border);
    let width = 1.2;
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let (cx, cy) = (x0 + r.w / 2.0, y0 + r.h / 2.0);
    let polygon = |points: Vec<(f32, f32)>, out: &mut Vec<Item>| {
        out.push(Item::path(closed(&points), width, border));
        out.insert(out.len() - 1, Item::polygon(points, fill));
    };
    match shape {
        Shape::Rect | Shape::Round | Shape::Subroutine | Shape::Stadium => {
            let radius = match shape {
                Shape::Round => 5.0,
                Shape::Stadium => r.h / 2.0,
                _ => 0.0,
            };
            out.push(Item::Fill {
                rect: r,
                color: fill,
                radius,
            });
            out.push(Item::path(outline(r, radius), width, border));
            if shape == Shape::Subroutine {
                let inset = r.h.min(16.0) / 2.0;
                out.push(Item::path(
                    vec![
                        Stroke::Line(x0 + inset, y0, x0 + inset, y1),
                        Stroke::Line(x1 - inset, y0, x1 - inset, y1),
                    ],
                    width,
                    border,
                ));
            }
        }
        Shape::Circle | Shape::DoubleCircle => {
            let rad = r.w / 2.0;
            out.push(Item::Fill {
                rect: r,
                color: fill,
                radius: rad,
            });
            let mut strokes = vec![Stroke::Circle(cx, cy, rad)];
            if shape == Shape::DoubleCircle {
                strokes.push(Stroke::Circle(cx, cy, rad - 4.0));
            }
            out.push(Item::path(strokes, width, border));
        }
        Shape::Diamond => polygon(vec![(cx, y0), (x1, cy), (cx, y1), (x0, cy)], out),
        Shape::Hexagon => {
            let d = r.h / 4.0;
            polygon(
                vec![
                    (x0 + d, y0),
                    (x1 - d, y0),
                    (x1, cy),
                    (x1 - d, y1),
                    (x0 + d, y1),
                    (x0, cy),
                ],
                out,
            );
        }
        Shape::Lean | Shape::LeanLeft | Shape::Trapezoid | Shape::TrapezoidDown => {
            let d = r.h * 0.3;
            let (tl, tr, br, bl) = match shape {
                Shape::Lean => (d, 0.0, -d, 0.0),
                Shape::LeanLeft => (0.0, -d, 0.0, d),
                Shape::Trapezoid => (d, -d, 0.0, 0.0),
                _ => (0.0, 0.0, -d, d),
            };
            // 每个角往里收多少：负的是右边往左收。
            let (tl, tr, br, bl) = (
                (x0 + tl.max(0.0), y0),
                (x1 + tr.min(0.0), y0),
                (x1 + br.min(0.0), y1),
                (x0 + bl.max(0.0), y1),
            );
            polygon(vec![tl, tr, br, bl], out);
        }
        Shape::Asym => {
            let n = r.h / 4.0;
            // 左边有个往里的缺口，不是凸的：分成一个矩形和两个三角形填色。
            out.push(Item::Fill {
                rect: Rect::new(x0 + n, y0, r.w - n, r.h),
                color: fill,
                radius: 0.0,
            });
            out.push(Item::polygon(
                vec![(x0, y0), (x0 + n, y0), (x0 + n, cy)],
                fill,
            ));
            out.push(Item::polygon(
                vec![(x0, y1), (x0 + n, cy), (x0 + n, y1)],
                fill,
            ));
            out.push(Item::path(
                closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0 + n, cy)]),
                width,
                border,
            ));
        }
        Shape::Cylinder => {
            let ry = cylinder_ry(r.w);
            let rx = r.w / 2.0;
            let (top, bottom) = (y0 + ry, y1 - ry);
            use std::f32::consts::{PI, TAU};
            out.push(Item::Fill {
                rect: Rect::new(x0, top, r.w, bottom - top),
                color: fill,
                radius: 0.0,
            });
            out.push(Item::polygon(ellipse(cx, top, rx, ry, 0.0, TAU, 32), fill));
            out.push(Item::polygon(
                ellipse(cx, bottom, rx, ry, 0.0, TAU, 32),
                fill,
            ));
            let mut strokes = polyline(&ellipse(cx, top, rx, ry, 0.0, TAU, 32));
            strokes.extend(polyline(&ellipse(cx, bottom, rx, ry, 0.0, PI, 16)));
            strokes.push(Stroke::Line(x0, top, x0, bottom));
            strokes.push(Stroke::Line(x1, top, x1, bottom));
            out.push(Item::path(strokes, width, border));
        }
        Shape::Start => out.push(Item::Fill {
            rect: r,
            color: look.line,
            radius: r.w / 2.0,
        }),
        Shape::End => {
            out.push(Item::path(
                vec![Stroke::Circle(cx, cy, r.w / 2.0)],
                width,
                look.line,
            ));
            let inner = r.inset(r.w * 0.22, r.h * 0.22);
            out.push(Item::Fill {
                rect: inner,
                color: look.line,
                radius: inner.w / 2.0,
            });
        }
        Shape::Bar => out.push(Item::Fill {
            rect: r,
            color: look.line,
            radius: 1.0,
        }),
    }
}

/// 从形状中心往 `toward` 的方向，和形状边缘的交点。
fn clip(shape: Shape, r: Rect, toward: (f32, f32)) -> (f32, f32) {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return (cx, cy);
    }
    let t = match shape {
        Shape::Diamond => 1.0 / (dx.abs() / hw + dy.abs() / hh),
        Shape::Circle | Shape::DoubleCircle | Shape::Start | Shape::End => {
            hw / (dx * dx + dy * dy).sqrt()
        }
        _ => {
            let tx = if dx.abs() > 1e-3 {
                hw / dx.abs()
            } else {
                f32::MAX
            };
            let ty = if dy.abs() > 1e-3 {
                hh / dy.abs()
            } else {
                f32::MAX
            };
            tx.min(ty)
        }
    };
    let t = t.min(1.0);
    (cx + dx * t, cy + dy * t)
}

/// 连到自己的线：从右边出去绕一圈回来。
fn self_loop(r: Rect, s: f32) -> Vec<(f32, f32)> {
    let (x, cy) = (r.right(), r.y + r.h / 2.0);
    let d = r.h / 4.0;
    vec![
        (x, cy - d),
        (x + s * 1.5, cy - d),
        (x + s * 1.5, cy + d),
        (x, cy + d),
    ]
}

/// 折线的拐角改成圆滑的曲线（拐角两边线段的中点之间用二次贝塞尔曲线连起来）。
fn smooth(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let mut out = vec![points[0]];
    for i in 1..points.len() - 1 {
        let a = if i == 1 {
            points[0]
        } else {
            mid(points[i - 1], points[i])
        };
        let b = if i == points.len() - 2 {
            points[i + 1]
        } else {
            mid(points[i], points[i + 1])
        };
        for k in 1..=8 {
            let t = k as f32 / 8.0;
            let u = 1.0 - t;
            let p = points[i];
            out.push((
                u * u * a.0 + 2.0 * u * t * p.0 + t * t * b.0,
                u * u * a.1 + 2.0 * u * t * p.1 + t * t * b.1,
            ));
        }
    }
    // 曲线的最后一点就是下一段的起点，去掉重复的点（线头要靠最后两点定方向）。
    out.dedup_by(|a, b| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
    out
}

/// 画线头（尖在 `tip`，线从 `from` 那边来），返回线该画到哪里为止。
///
/// `len` 是线头的长度（逻辑像素）。
pub(super) fn arrow_head(
    head: Head,
    tip: (f32, f32),
    from: (f32, f32),
    len: f32,
    color: Color,
    out: &mut Vec<Item>,
) -> (f32, f32) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let d = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / d, dy / d);
    let back = |k: f32| (tip.0 - ux * len * k, tip.1 - uy * len * k);
    match head {
        Head::Arrow => {
            let base = back(1.0);
            let (px, py) = (-uy * len * 0.45, ux * len * 0.45);
            out.push(Item::polygon(
                vec![tip, (base.0 + px, base.1 + py), (base.0 - px, base.1 - py)],
                color,
            ));
            back(0.8)
        }
        Head::Circle => {
            let r = len * 0.35;
            let c = back(0.35);
            out.push(Item::Fill {
                rect: Rect::new(c.0 - r, c.1 - r, r * 2.0, r * 2.0),
                color,
                radius: r,
            });
            back(0.7)
        }
        Head::Cross => {
            let c = back(0.5);
            let k = len * 0.35;
            out.push(Item::path(
                vec![
                    Stroke::Line(c.0 - k, c.1 - k, c.0 + k, c.1 + k),
                    Stroke::Line(c.0 - k, c.1 + k, c.0 + k, c.1 - k),
                ],
                len * 0.15,
                color,
            ));
            c
        }
        Head::None => tip,
    }
}

/// 虚线：沿折线画 `on` 长、空 `off` 长（都是逻辑像素）。
pub(super) fn dashed(points: &[(f32, f32)], on: f32, off: f32) -> Vec<Stroke> {
    let mut out = Vec::new();
    let mut drawing = true;
    let mut left = on;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let mut at = 0.0;
        while at < len {
            let step = left.min(len - at);
            if drawing {
                let p = |t: f32| (a.0 + (b.0 - a.0) * t / len, a.1 + (b.1 - a.1) * t / len);
                let (p0, p1) = (p(at), p(at + step));
                out.push(Stroke::Line(p0.0, p0.1, p1.0, p1.1));
            }
            at += step;
            left -= step;
            if left <= 1e-3 {
                drawing = !drawing;
                left = if drawing { on } else { off };
            }
        }
    }
    out
}
