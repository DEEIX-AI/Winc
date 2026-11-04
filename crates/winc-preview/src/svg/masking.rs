//! 裁剪和蒙版：`<clipPath>` 与 `<mask>` 元素。

use super::*;

impl<'a, 'input> Parser<'a, 'input> {
    /// 解析 `<clipPath>`：把其中的形状（包括 `<use>` 引用的）合并为一个裁剪区域。
    ///
    /// 数量单位取外框比例；元素没有外框时返回 `None`。
    pub(super) fn clip(
        &mut self,
        node: Node<'a, 'input>,
        ctm: Affine,
        bbox: Option<[f32; 4]>,
        depth: usize,
    ) -> Option<Arc<Clip>> {
        // 嵌套超过 4 层或正在展开自身时按空裁剪处理：元素整个不可见，也不会无限递归。
        if depth > 4 || self.active.contains(&node) {
            return Some(Arc::new(Clip {
                shapes: Vec::new(),
                clip: None,
            }));
        }
        self.active.push(node);
        let units = units(node, "clipPathUnits", false, bbox);
        let result = units.map(|units| {
            let own = self.own(node);
            let base = own.transform.then(units).then(ctm);
            let props = self.inherited(node);
            let mut shapes = Vec::new();
            for child in node.children().filter(Node::is_element) {
                self.clip_shape(child, &props, base, &mut shapes, 0);
            }
            // 裁剪路径自身又带有 clip-path。
            let clip = own
                .clip_path
                .as_deref()
                .and_then(|id| self.ids.get(id).copied())
                .filter(|n| n.tag_name().name() == "clipPath")
                .and_then(|n| self.clip(n, ctm, bbox, depth + 1));
            Arc::new(Clip { shapes, clip })
        });
        self.active.pop();
        result
    }

    /// 把 `node` 的几何追加到裁剪区域：展开 `<use>`，支持基本形状，文字不参与裁剪。
    fn clip_shape(
        &mut self,
        node: Node<'a, 'input>,
        parent: &Props,
        ctm: Affine,
        out: &mut Vec<(Vec<Command>, Affine, Fill)>,
        depth: usize,
    ) {
        if depth > MAX_DEPTH || !self.effect_step() {
            return;
        }
        let declared = self.declared(node);
        let own = Self::own_from(&declared);
        let props = self.props_from(&declared, parent);
        if !own.shown || !props.visible {
            return;
        }
        let ctm = own.transform.then(ctm);
        match node.tag_name().name() {
            "use" => {
                if let Some(target) = href(node).and_then(|id| self.ids.get(id).copied()) {
                    let (vw, vh) = self.viewport;
                    let x = len_attr(node, "x", vw);
                    let y = len_attr(node, "y", vh);
                    self.clip_shape(
                        target,
                        &props,
                        Affine::translate(x, y).then(ctm),
                        out,
                        depth + 1,
                    );
                }
            }
            "text" => self.miss(winc_i18n::t!("preview-svg-missing-clip-text")),
            _ => {
                if let Some(path) = self.geometry(node) {
                    out.push((path, ctm, props.clip_rule));
                }
            }
        }
    }

    /// 解析 `<mask>`：绘制里面的内容作为蒙版。
    ///
    /// 单位取外框比例而元素没有外框时返回 `None`。
    pub(super) fn mask(
        &mut self,
        node: Node<'a, 'input>,
        ctm: Affine,
        bbox: Option<[f32; 4]>,
        depth: usize,
    ) -> Option<Arc<MaskRef>> {
        // 嵌套超过 4 层或正在展开自身时按没有蒙版处理，也不会无限递归。
        if depth > 4 || self.active.contains(&node) {
            return None;
        }
        // 蒙版的范围：默认是外框向四周各扩大 10%。
        let region_units = units(node, "maskUnits", true, bbox)?;
        let by_box = region_units != Affine::IDENTITY;
        let (vw, vh) = self.viewport;
        let coord = |name: &str, default: f32, reference: f32| match node.attribute(name) {
            Some(v) if by_box => fraction(v).unwrap_or(default),
            Some(v) => length(v, reference).unwrap_or(default * reference),
            None if by_box => default,
            None => default * reference,
        };
        let (x, y) = (coord("x", -0.1, vw), coord("y", -0.1, vh));
        let (w, h) = (coord("width", 1.2, vw), coord("height", 1.2, vh));
        let region = if w > 0.0 && h > 0.0 {
            rect_clip(x, y, w, h, region_units.then(ctm))
        } else {
            Arc::new(Clip {
                shapes: Vec::new(),
                clip: None,
            })
        };

        self.active.push(node);
        let content_units = units(node, "maskContentUnits", false, bbox);
        let own = self.own(node);
        let mut items = Vec::new();
        if let Some(units) = content_units {
            let props = self.inherited(node);
            self.inside_effect += 1;
            self.children(node, &props, units.then(ctm), &mut items, 1);
            self.inside_effect -= 1;
        }
        let mask = own
            .mask
            .as_deref()
            .and_then(|id| self.ids.get(id).copied())
            .filter(|n| n.tag_name().name() == "mask")
            .and_then(|n| self.mask(n, ctm, bbox, depth + 1));
        self.active.pop();
        Some(Arc::new(MaskRef {
            items,
            region,
            luminance: !own.mask_alpha,
            mask,
        }))
    }
}

/// 返回只保留矩形 (x, y, w, h) 以内部分的裁剪；矩形的坐标经 `ctm` 变换到 SVG 根坐标。
pub(super) fn rect_clip(x: f32, y: f32, w: f32, h: f32, ctm: Affine) -> Arc<Clip> {
    let mut rect = Vec::new();
    rounded_rect(&mut rect, x, y, w, h, 0.0, 0.0);
    Arc::new(Clip {
        shapes: vec![(rect, ctm, Fill::NonZero)],
        clip: None,
    })
}
