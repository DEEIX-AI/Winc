//! 颜料：纯色、线性和径向渐变、图案。

use super::*;

impl<'a, 'input> Parser<'a, 'input> {
    /// 返回填充或描边使用的颜料；没有可用颜料时返回 `None`。
    ///
    /// `bbox` 是形状的外框（x、y、宽、高，用户坐标），渐变和图案按外框比例取单位时需要用到。
    pub(super) fn paint(
        &mut self,
        spec: &PaintSpec,
        alpha: f32,
        props: &Props,
        bbox: [f32; 4],
    ) -> Option<Paint> {
        let solid = |c: [u8; 4]| {
            let a = (f32::from(c[3]) * alpha).round() as u8;
            (a > 0).then_some(Paint::Solid([c[0], c[1], c[2], a]))
        };
        match spec {
            PaintSpec::None => None,
            PaintSpec::Color(c) => solid(*c),
            PaintSpec::CurrentColor => solid(props.color),
            PaintSpec::Url(id, fallback) => {
                let Some(node) = self.ids.get(id.as_str()).copied() else {
                    return self.paint(fallback, alpha, props, bbox);
                };
                match node.tag_name().name() {
                    "linearGradient" | "radialGradient" => self.gradient(node, alpha, props, bbox),
                    "pattern" => match self.pattern(node, alpha, bbox) {
                        Some(paint) => Some(paint),
                        None => self.paint(fallback, alpha, props, bbox),
                    },
                    _ => self.paint(fallback, alpha, props, bbox),
                }
            }
        }
    }

    /// 解析 `<linearGradient>` 或 `<radialGradient>`，返回渐变颜料。
    /// 属性和色标可以沿 `href` 继承，颜色表按引用它的形状缓存，同一个渐变只解析一次。
    /// 渐变坐标按 `gradientUnits` 取外框比例或用户坐标。
    fn gradient(
        &mut self,
        node: Node<'a, 'input>,
        alpha: f32,
        props: &Props,
        bbox: [f32; 4],
    ) -> Option<Paint> {
        // `href` 指向的渐变上的属性和色标可以继承；`8` 是继承链的最大回溯层数，避免链成环或链过长。
        let mut chain = vec![node];
        while chain.len() < 8 {
            let last = chain[chain.len() - 1];
            let next = href(last)
                .and_then(|id| self.ids.get(id).copied())
                .filter(|n| !chain.contains(n));
            match next {
                Some(n) => chain.push(n),
                None => break,
            }
        }
        let attr = |name: &str| chain.iter().find_map(|n| n.attribute(name));
        let stops_from = *chain
            .iter()
            .find(|n| n.children().any(|c| c.tag_name().name() == "stop"))?;
        let key = (stops_from.id(), alpha.to_bits(), props.color);
        let ramp = match self.ramps.get(&key) {
            Some(ramp) => ramp.clone(),
            None => {
                let ramp = self.ramp(stops_from, alpha, props.color);
                self.ramps.insert(key, ramp.clone());
                ramp
            }
        };
        let table = match ramp? {
            Ramp::Solid(c) => return Some(Paint::Solid(c)),
            Ramp::Table(table) => table,
        };

        let user_space = attr("gradientUnits") == Some("userSpaceOnUse");
        let (vw, vh) = self.viewport;
        let diagonal = self.diagonal();
        // 单位取外框比例时，数值和百分比都表示 0..1 的比例。
        let coord = |name: &str, default: f32, reference: f32| -> f32 {
            match attr(name).map(str::trim) {
                Some(v) if !user_space => match v.strip_suffix('%') {
                    Some(p) => p.parse::<f32>().map_or(default, |p| p / 100.0),
                    None => v.parse().unwrap_or(default),
                },
                Some(v) => length(v, reference).unwrap_or(default * reference),
                None if user_space => default * reference,
                None => default,
            }
        };
        let radial = node.tag_name().name() == "radialGradient";
        let geometry = if radial {
            let (cx, cy) = (coord("cx", 0.5, vw), coord("cy", 0.5, vh));
            let r = coord("r", 0.5, diagonal);
            let fx = if attr("fx").is_some() {
                coord("fx", 0.5, vw)
            } else {
                cx
            };
            let fy = if attr("fy").is_some() {
                coord("fy", 0.5, vh)
            } else {
                cy
            };
            let fr = coord("fr", 0.0, diagonal);
            [cx, cy, r, fx, fy, fr]
        } else {
            [
                coord("x1", 0.0, vw),
                coord("y1", 0.0, vh),
                coord("x2", 1.0, vw),
                coord("y2", 0.0, vh),
                0.0,
                0.0,
            ]
        };
        let mut transform = attr("gradientTransform").map_or(Affine::IDENTITY, transform);
        if !user_space {
            if bbox[2] <= 0.0 || bbox[3] <= 0.0 {
                return None;
            }
            transform = transform.then(Affine([bbox[2], 0.0, 0.0, bbox[3], bbox[0], bbox[1]]));
        }
        let spread = match attr("spreadMethod") {
            Some("reflect") => Spread::Reflect,
            Some("repeat") => Spread::Repeat,
            _ => Spread::Pad,
        };
        Some(Paint::Gradient(Arc::new(Gradient {
            radial,
            geometry,
            transform,
            spread,
            table,
        })))
    }

    /// 解析 `node` 里的 `<stop>`，返回渐变的颜色表。
    ///
    /// 每个色标的透明度再乘以 `alpha`，`currentColor` 取 `current`；色标偏移量限制在
    /// 0 到 1 之间，且不小于前一个色标的偏移量。
    ///
    /// 没有色标、或只有一个完全透明的色标时返回 `None`。
    fn ramp(&self, node: Node<'a, 'input>, alpha: f32, current: [u8; 4]) -> Option<Ramp> {
        let mut stops: Vec<(f32, [u8; 4])> = Vec::new();
        for stop in node.children().filter(|c| c.tag_name().name() == "stop") {
            let mut offset = stop.attribute("offset").map_or(0.0, |o| {
                let o = o.trim();
                match o.strip_suffix('%') {
                    Some(p) => p.parse::<f32>().unwrap_or(0.0) / 100.0,
                    None => o.parse().unwrap_or(0.0),
                }
            });
            offset = offset
                .clamp(0.0, 1.0)
                .max(stops.last().map_or(0.0, |s| s.0));
            let mut stop_color = [0, 0, 0, 255];
            let mut stop_alpha = 1.0;
            for (name, value) in self.declared(stop) {
                match name.as_ref() {
                    "stop-color" => {
                        stop_color = match value.trim() {
                            "currentColor" => current,
                            v => color(v).unwrap_or(stop_color),
                        }
                    }
                    "stop-opacity" => stop_alpha = opacity(value),
                    _ => {}
                }
            }
            stop_color[3] = (f32::from(stop_color[3]) * stop_alpha * alpha).round() as u8;
            stops.push((offset, stop_color));
        }
        match stops.as_slice() {
            [] => None,
            [(_, c)] => (c[3] > 0).then_some(Ramp::Solid(*c)),
            _ => Some(Ramp::Table(Arc::new(gradient_table(&stops)))),
        }
    }

    /// 解析图案（pattern 元素）。
    ///
    /// 属性和内容可以从 `href` 指向的图案继承；图块内容为空或外框无效时返回 `None`。
    /// 图案的图块大小按外框比例或用户坐标换算，结果为单位方形的尺寸。
    fn pattern(&mut self, node: Node<'a, 'input>, alpha: f32, bbox: [f32; 4]) -> Option<Paint> {
        if self.active.contains(&node) || self.patterns >= MAX_PATTERN_NESTING {
            return None;
        }
        if !self.effect_step() {
            return None;
        }
        // 图案的属性和内容同样可以通过 `href` 从另一个图案继承；`8` 是继承链的最大回溯层数。
        let mut chain = vec![node];
        while chain.len() < 8 {
            let next = href(chain[chain.len() - 1])
                .and_then(|id| self.ids.get(id).copied())
                .filter(|n| n.tag_name().name() == "pattern" && !chain.contains(n));
            match next {
                Some(n) => chain.push(n),
                None => break,
            }
        }
        let attr = |name: &str| chain.iter().find_map(|n| n.attribute(name));
        let content = *chain
            .iter()
            .find(|n| n.children().any(|c| c.is_element()))?;
        let bbox_units = attr("patternUnits") != Some("userSpaceOnUse");
        let content_bbox = attr("patternContentUnits") == Some("objectBoundingBox");
        let (vw, vh) = self.viewport;
        let coord = |name: &str, reference: f32| -> f32 {
            match attr(name) {
                Some(v) if bbox_units => fraction(v).unwrap_or(0.0),
                Some(v) => length(v, reference).unwrap_or(0.0),
                None => 0.0,
            }
        };
        let (mut x, mut y, mut w, mut h) = (
            coord("x", vw),
            coord("y", vh),
            coord("width", vw),
            coord("height", vh),
        );
        if bbox_units {
            if bbox[2] <= 0.0 || bbox[3] <= 0.0 {
                return None;
            }
            (x, y, w, h) = (
                bbox[0] + x * bbox[2],
                bbox[1] + y * bbox[3],
                w * bbox[2],
                h * bbox[3],
            );
        }
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        // 图块内容的变换：有 `viewBox` 时按 `viewBox` 放进图块；没有时按单位换算，
        // 单位取外框比例时乘外框大小。
        let view = chain.iter().find_map(|n| view_box(*n));
        let inner = match view {
            Some(v) => fit(v, aspect(attr("preserveAspectRatio")), w, h),
            None if content_bbox => Affine([bbox[2], 0.0, 0.0, bbox[3], 0.0, 0.0]),
            None => Affine::IDENTITY,
        };
        let pattern_transform = attr("patternTransform").map_or(Affine::IDENTITY, transform);
        self.active.push(node);
        self.patterns += 1;
        self.inside_effect += 1;
        let props = self.inherited(content);
        let mut items = Vec::new();
        self.children(content, &props, inner, &mut items, 1);
        self.inside_effect -= 1;
        self.patterns -= 1;
        self.active.pop();
        if items.is_empty() {
            return None;
        }
        Some(Paint::Pattern(Arc::new(Pattern {
            size: (w, h),
            transform: Affine::translate(x, y).then(pattern_transform),
            items,
            opacity: alpha,
        })))
    }
}

/// 返回渐变在 0.1 上 256 档的颜色表：按不预乘的颜色插值，结果存为预乘颜色。
fn gradient_table(stops: &[(f32, [u8; 4])]) -> [[u8; 4]; 256] {
    std::array::from_fn(|i| {
        let t = i as f32 / 255.0;
        let after = stops.iter().position(|s| s.0 >= t).unwrap_or(stops.len());
        let c = if after == 0 {
            stops[0].1
        } else if after == stops.len() {
            stops[stops.len() - 1].1
        } else {
            let (o0, c0) = stops[after - 1];
            let (o1, c1) = stops[after];
            let k = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
            std::array::from_fn(|j| {
                (f32::from(c0[j]) + (f32::from(c1[j]) - f32::from(c0[j])) * k).round() as u8
            })
        };
        premultiply(c)
    })
}

/// 返回预乘 alpha 的颜色：RGB 各分量乘以透明度。
pub(super) fn premultiply([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let m = |c: u8| ((u32::from(c) * u32::from(a) + 127) / 255) as u8;
    [m(r), m(g), m(b), a]
}
