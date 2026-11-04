//! 形状：路径和基本形状的几何、填充和描边。

use super::*;

impl<'a, 'input> Parser<'a, 'input> {
    /// 返回形状元素的路径，坐标沿用元素自身坐标系。
    /// 不是形状元素、或没有有效几何（半径为 0、宽高非正、缺少 `d`）时返回 `None`。
    pub(super) fn geometry(&self, node: Node) -> Option<Vec<Command>> {
        let (vw, vh) = self.viewport;
        let diagonal = self.diagonal();
        let num = |name: &str, reference: f32| len_attr(node, name, reference);
        let mut path = Vec::new();
        match node.tag_name().name() {
            "path" => path = path_data(node.attribute("d")?),
            "rect" => {
                let (x, y) = (num("x", vw), num("y", vh));
                let (w, h) = (num("width", vw), num("height", vh));
                if w <= 0.0 || h <= 0.0 {
                    return None;
                }
                let rx = node
                    .attribute("rx")
                    .and_then(|v| length(v, vw))
                    .filter(|r| *r >= 0.0);
                let ry = node
                    .attribute("ry")
                    .and_then(|v| length(v, vh))
                    .filter(|r| *r >= 0.0);
                let (rx, ry) = match (rx, ry) {
                    (Some(rx), Some(ry)) => (rx, ry),
                    (Some(r), None) | (None, Some(r)) => (r, r),
                    (None, None) => (0.0, 0.0),
                };
                rounded_rect(&mut path, x, y, w, h, rx.min(w / 2.0), ry.min(h / 2.0));
            }
            "circle" => {
                let r = num("r", diagonal);
                if r <= 0.0 {
                    return None;
                }
                ellipse(&mut path, num("cx", vw), num("cy", vh), r, r);
            }
            "ellipse" => {
                let rx = node.attribute("rx").and_then(|v| length(v, vw));
                let ry = node.attribute("ry").and_then(|v| length(v, vh));
                // SVG 2：只写了一个半径时，另一个半径取相同的值（`auto`）。
                let (rx, ry) = match (rx, ry) {
                    (Some(rx), Some(ry)) => (rx, ry),
                    (Some(r), None) | (None, Some(r)) => (r, r),
                    (None, None) => return None,
                };
                if rx <= 0.0 || ry <= 0.0 {
                    return None;
                }
                ellipse(&mut path, num("cx", vw), num("cy", vh), rx, ry);
            }
            "line" => {
                path.push(Command::MoveTo([num("x1", vw), num("y1", vh)].into()));
                path.push(Command::LineTo([num("x2", vw), num("y2", vh)].into()));
            }
            tag @ ("polyline" | "polygon") => {
                let points = numbers(node.attribute("points")?);
                for (i, p) in points.as_chunks::<2>().0.iter().enumerate() {
                    let point = [p[0], p[1]].into();
                    path.push(if i == 0 {
                        Command::MoveTo(point)
                    } else {
                        Command::LineTo(point)
                    });
                }
                if tag == "polygon" && !path.is_empty() {
                    path.push(Command::Close);
                }
            }
            _ => return None,
        }
        (!path.is_empty()).then_some(path)
    }

    /// 解析形状的填充与描边，并追加一个形状条目。
    /// 填充与描边都不可用时不追加，`paint-order` 决定二者的绘制先后。
    pub(super) fn shape(
        &mut self,
        node: Node,
        path: Vec<Command>,
        props: &Props,
        own: &Own,
        ctm: Affine,
        out: &mut Vec<Item>,
    ) {
        if !props.visible {
            return;
        }
        let bounds = path_bounds(&path);
        let fill = self
            .paint(&props.fill, props.fill_opacity, props, bounds)
            .map(|p| (p, props.fill_rule))
            // 线段没有面积，不填充。
            .filter(|_| node.tag_name().name() != "line");
        let stroke = self.stroke(props, own, bounds);
        if fill.is_none() && stroke.is_none() {
            return;
        }
        self.shapes += 1;
        out.push(Item::Shape(Shape {
            path,
            bounds,
            transform: ctm,
            fill,
            stroke,
            stroke_first: props.stroke_first,
        }));
    }

    /// 返回描边使用的颜料和描边样式。
    /// 线宽不大于 0 或颜料不可用时返回 `None`；`non_scaling` 表示线宽按画布像素计。
    pub(super) fn stroke(
        &mut self,
        props: &Props,
        own: &Own,
        bounds: [f32; 4],
    ) -> Option<(Paint, StrokeStyle)> {
        if props.stroke_width <= 0.0 {
            return None;
        }
        let paint = self.paint(&props.stroke, props.stroke_opacity, props, bounds)?;
        Some((
            paint,
            StrokeStyle {
                width: props.stroke_width,
                cap: props.cap,
                join: props.join,
                miter: props.miter,
                dashes: Arc::clone(&props.dashes),
                offset: props.dash_offset,
                non_scaling: own.non_scaling,
            },
        ))
    }
}
