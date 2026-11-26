//! 真实粗体字重的按需解析与加载（可变字体 `wght` 轴、系统轮廓命名实例与同族静态粗体文件）。

use super::*;

/// 认定找到真实粗体字形的最小字重增量（例如 Regular 400 → Semibold 600 增量为 200）。
const MIN_BOLD_GAIN: f32 = 150.0;

/// 在系统轮廓字体文件 `data`（如 macOS `PingFangUI.ttc`）中查找与当前字族前缀一致的更粗命名实例。
fn system_bold_instance<'a>(
    data: &'a [u8],
    postscript: Option<&str>,
    fallback_font: &FontRef<'a>,
    base_weight: f32,
    step: f32,
) -> Option<(FontRef<'a>, String, Vec<i16>, f32)> {
    let ps = postscript
        .map(str::to_owned)
        .or_else(|| postscript_name(fallback_font))?;
    let prefix = ps.split_once('-').map_or(ps.as_str(), |(p, _)| p);
    let wanted = base_weight + step;
    let count = swash::FontDataRef::new(data).map_or(1, |d| d.len());
    let mut best: Option<(FontRef<'a>, String, Vec<i16>, f32)> = None;
    for font in (0..count).filter_map(|i| FontRef::from_index(data, i)) {
        for inst in font.instances() {
            let Some(name) = instance_postscript(&font, inst.index()) else {
                continue;
            };
            let inst_prefix = name.split_once('-').map_or(name.as_str(), |(p, _)| p);
            if inst_prefix != prefix {
                continue;
            }
            let values: Vec<f32> = inst.values().collect();
            let weight = font
                .variations()
                .position(|v| v.tag() == WGHT)
                .and_then(|at| values.get(at).copied())
                .unwrap_or(REGULAR);
            let gained = weight - base_weight;
            if gained < MIN_BOLD_GAIN || weight > wanted + 100.0 {
                continue;
            }
            let closer = best.as_ref().is_none_or(|(_, _, _, best_gain)| {
                (step - gained).abs() < (step - *best_gain).abs()
            });
            if closer {
                let coords: Vec<i16> = inst.normalized_coords().collect();
                best = Some((font, name, coords, gained));
            }
        }
    }
    best
}

impl Face {
    /// 按真实粗体打开 `file`（相对常规字重偏移 `base_shift` 增加粗体字重，优先走可变字体
    /// `wght` 轴、同族粗体文件或系统轮廓字体的粗体命名实例）。
    ///
    /// 仅当确实获得明显更重的真实字形（字重增量不少于 [`MIN_BOLD_GAIN`]）时返回 `Some`
    /// （且 `embolden == 0.0`），否则返回 `None` 交由调用方回退到模拟加粗。
    pub(crate) fn open_bold(file: &FontFile, base_shift: i16) -> Option<Face> {
        let data = load(&file.path)?;
        let selected = select(&data, file.postscript.as_deref())?;
        let font = selected.font;
        let (offset, key) = (font.offset, font.key);
        let base_shift = f32::from(base_shift);

        if selected.system {
            let base_weight = selected
                .instance
                .as_ref()
                .map_or(REGULAR, |inst| inst.weight(&font))
                + base_shift;
            let (bold_font, bold_ps, coords, gained) = system_bold_instance(
                &data,
                file.postscript.as_deref(),
                &font,
                base_weight,
                BOLD_STEP,
            )?;
            if gained < MIN_BOLD_GAIN {
                return None;
            }
            let system = SystemOutlines::new(&file.path, &bold_ps, &bold_font)?;
            let (bold_offset, bold_key) = (bold_font.offset, bold_font.key);
            return Some(Face {
                data,
                offset: bold_offset,
                key: bold_key,
                coords,
                system: Some(system),
                embolden: 0.0,
            });
        }

        if let Some((min, max)) = weight_range(&font) {
            let (coords, gained) = match selected.instance {
                Some(instance) => {
                    let base = (instance.weight(&font) + base_shift).clamp(min, max);
                    let (coords, reached) =
                        instance.shifted(&font, base_shift + BOLD_STEP, (min, max));
                    (coords, reached - base)
                }
                None => {
                    let base = (REGULAR + base_shift).clamp(min, max);
                    let target = (base + BOLD_STEP).clamp(min, max);
                    let coords = font
                        .variations()
                        .normalized_coords([("wght", target)])
                        .collect();
                    (coords, target - base)
                }
            };
            if gained < MIN_BOLD_GAIN {
                return None;
            }
            return Some(Face {
                data,
                offset,
                key,
                coords,
                system: None,
                embolden: 0.0,
            });
        }

        let (sibling_file, reached) = sibling(file, &font, BOLD_STEP)?;
        if reached < MIN_BOLD_GAIN {
            return None;
        }
        let mut face = Self::open_as(&sibling_file, Some(0))?;
        face.embolden = 0.0;
        Some(face)
    }
}

impl Fonts {
    /// 返回粗体字符 `c` 对应的（槽位，字形 id，是否需要模拟加粗）。
    ///
    /// 优先为常规命中的槽位按需加载真粗体（可变字体 `wght` 轴、同族粗体文件或系统粗体命名实例）；
    /// 当真粗体不可用或缺少该字符时，回退到常规槽位并置 `faux_bold = true`。
    pub(crate) fn resolve_bold(&mut self, kind: FontKind, c: char) -> (u8, u16, bool) {
        let kind = self.effective(kind);
        if (c as u32) < 128
            && let Some(hit) = self.ascii_bold[kind as usize][c as usize]
        {
            return hit;
        }
        if let Some(&hit) = self.resolved_bold.get(&(kind, c)) {
            return hit;
        }
        let (slot, glyph) = self.resolve(kind, c);
        let result = if glyph == 0 || self.is_emoji_slot(slot) {
            (slot, glyph, false)
        } else if let Some(bold_slot) = self.ensure_bold_slot(slot)
            && let Some(face) = self.face(bold_slot)
            && let bold_glyph = face.font().charmap().map(c)
            && bold_glyph != 0
        {
            (bold_slot, bold_glyph, false)
        } else {
            (slot, glyph, true)
        };
        if (c as u32) < 128 {
            self.ascii_bold[kind as usize][c as usize] = Some(result);
        } else {
            if self.resolved_bold.len() >= RESOLVED_ENTRIES {
                self.resolved_bold = FastMap::default();
            }
            self.resolved_bold.insert((kind, c), result);
        }
        result
    }

    /// 按需为常规槽位 `slot` 加载对应的真粗体槽位；不存在或无法加载时记住 `Unavailable`。
    fn ensure_bold_slot(&mut self, slot: u8) -> Option<u8> {
        let idx = slot as usize;
        match *self.bold_of.get(idx)? {
            BoldSlot::Ready(bold) => return Some(bold),
            BoldSlot::Unavailable => return None,
            BoldSlot::Pending => {}
        }
        let (file, base_shift) = match self.slots.get(idx) {
            Some(Slot::Ready(_, Some(file), shift)) => (file.clone(), *shift),
            _ => {
                self.bold_of[idx] = BoldSlot::Unavailable;
                return None;
            }
        };
        if self.slots.len() >= u8::MAX as usize {
            self.bold_of[idx] = BoldSlot::Unavailable;
            return None;
        }
        match Face::open_bold(&file, base_shift) {
            Some(face) => {
                let bold_slot = self.slots.len() as u8;
                self.slots.push(Slot::Ready(face, None, base_shift));
                self.bold_of.push(BoldSlot::Unavailable);
                if italic_slot(slot) {
                    self.italic_bold_slots.push(bold_slot);
                }
                self.bold_of[idx] = BoldSlot::Ready(bold_slot);
                Some(bold_slot)
            }
            None => {
                self.bold_of[idx] = BoldSlot::Unavailable;
                None
            }
        }
    }
}
