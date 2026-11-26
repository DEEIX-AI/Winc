//! 长文档的局部增量排版：跨出 [`Near`] 缓冲时的就地刷新与单块实时输入就地修补。

use std::collections::HashMap;
use std::sync::Arc;

use winc_render::Frame;
use winc_typeset::{Colors, Cx, Env, LaidBlock, Memo, Out, Raw, margins};

use crate::app::{App, Focus};

use super::{Key, Laid, Near, SHED_BLOCKS, live_key};

/// 按 `app` 的当前状态构造光标所在顶层块的原文参数，供排版和就地局部更新共用。
pub(super) type LiveRaw = (
    usize,
    Arc<str>,
    std::ops::Range<usize>,
    usize,
    usize,
    bool,
    Option<Arc<str>>,
);

/// 判断块或其子块中是否包含带展开/收起状态的块（`<details>` 折叠块或可能折叠的 Mermaid 图）。
pub(super) fn block_has_toggle(block: &winc_preview::markdown::Block) -> bool {
    use winc_preview::markdown::Block;
    match block {
        Block::Details { .. } => true,
        Block::Code { lang, .. } => lang == "mermaid",
        Block::Quote(_, inner) | Block::Footnote(_, inner) => inner.iter().any(block_has_toggle),
        Block::List(_, items) => items
            .iter()
            .any(|item| item.blocks.iter().any(block_has_toggle)),
        _ => false,
    }
}

pub(super) fn live_raw(
    app: &App,
    live: Option<&crate::app::preview::markdown::LiveLeaf>,
) -> Option<LiveRaw> {
    let live = live?;
    let rope = app.editor.active()?.text().rope()?;
    let snap = |b: usize| rope.char_to_byte(rope.byte_to_char(b.min(rope.len_bytes())));
    let (start, end) = (snap(live.range.start), snap(live.range.end));
    let text: Arc<str> = rope.byte_slice(start..end).to_string().into();
    let leaf =
        snap(live.leaf.start).saturating_sub(start)..snap(live.leaf.end).saturating_sub(start);
    let caret = snap(live.at).saturating_sub(start);
    let preedit = app.preedit_at(Focus::Editor);
    let preedit = ((live.prose || live.table) && !preedit.is_empty()).then(|| Arc::from(preedit));
    let parsed_end = live.parsed_end.saturating_sub(start);
    Some((
        live.block, text, leaf, parsed_end, caret, live.prose, preedit,
    ))
}

pub(super) fn prepare_memo(memo: &mut Memo, key: Key) {
    memo.prepare(winc_typeset::memo_key(|h| {
        use std::hash::Hash;
        for v in [key.width, key.base, key.tall] {
            v.to_bits().hash(h);
        }
        for c in [key.text, key.background] {
            [c.r, c.g, c.b, c.a].map(f32::to_bits).hash(h);
        }
        key.images.hash(h);
        key.numbers.hash(h);
    }));
}

pub(super) fn layout_one_block(
    cx: &mut Cx<'_, '_>,
    block: &winc_preview::markdown::Block,
    width: f32,
    env: Env,
    raw: Option<&LiveRaw>,
) -> LaidBlock {
    let mut out = Out::default();
    cx.text.clear();
    cx.chars = 0;
    cx.memo.take_touched();
    cx.raw = raw.map(|(_, text, leaf, parsed_end, caret, prose, preedit)| {
        Raw::new(
            Arc::clone(text),
            leaf.clone(),
            *parsed_end,
            *caret,
            *prose,
            preedit.clone(),
        )
    });
    cx.preedit = false;
    cx.cell = None;
    let mut flow = cx.block(block, 0.0, 0.0, width, env, &mut out);
    // 光标所在的叶子块在块里没有找到（写在列表里的公式这类没有原文对照的块）：整个
    // 顶层块显示原文。
    if let Some(raw) = cx.raw.as_mut().filter(|r| !r.done) {
        raw.leaf = 0..raw.block.len();
        raw.parsed_end = raw.block.len();
        raw.kind = raw.guess();
        out = Out::default();
        cx.text.clear();
        cx.chars = 0;
        flow = cx.raw_with_preview(block, 0.0, 0.0, width, env, &mut out);
    }
    cx.raw = None;
    let mut laid = LaidBlock::new(
        flow.height,
        out.items,
        out.anchors,
        cx.text.as_str().into(),
        cx.chars,
        cx.memo.take_touched(),
    );
    laid.preedit = cx.preedit;
    laid.scrolls = out.scrolls;
    laid.preview = out.preview;
    laid
}

impl Laid {
    /// 返回与 `near`（视口纵坐标区间 + 块序号区间）相交的顶层块范围列表（O(log N) 二分定位）。
    fn near_ranges(&self, near: &Near) -> Vec<std::ops::Range<usize>> {
        let len = self.blocks.len();
        let mut ranges = Vec::with_capacity(near.blocks.len() + 1);
        let row_range = self.blocks_in(near.rows.0, near.rows.1);
        if !row_range.is_empty() {
            ranges.push(row_range);
        }
        for r in &near.blocks {
            let s = r.start.min(len);
            let e = r.end.min(len);
            if s < e {
                ranges.push(s..e);
            }
        }
        ranges
    }

    /// 纯滚动或跳转跨出 [`Near`] 缓冲范围时的就地刷新：只释放离开范围的块、只重排新进入且已释放的块，
    /// 避免重建整篇文档的块表与哈希表。
    pub(crate) fn refresh_near(
        &mut self,
        app: &mut App,
        frame: &mut Frame<'_>,
        parsed: &winc_preview::markdown::Markdown,
        live: Option<&crate::app::preview::markdown::LiveLeaf>,
        near: Near,
    ) {
        if self.blocks.len() <= SHED_BLOCKS || self.blocks.len() != parsed.blocks.len() {
            return;
        }
        let old_ranges = self
            .near
            .as_ref()
            .map(|n| self.near_ranges(n))
            .unwrap_or_default();
        let new_ranges = self.near_ranges(&near);
        let live_block = live.map(|l| l.block);

        // 1. 释放已离开新 `near` 范围的旧块。
        for range in old_ranges {
            for i in range {
                if live_block == Some(i) {
                    continue;
                }
                if !near.holds(i, self.tops[i], self.blocks[i].1.height)
                    && !self.blocks[i].1.is_shed()
                {
                    let shed = self.blocks[i].1.shed();
                    self.blocks[i].1 = Arc::new(shed);
                }
            }
        }

        // 2. 仅对新 `near` 范围内已释放的块执行局部排版。
        let needs_layout = new_ranges
            .iter()
            .any(|r| self.blocks[r.clone()].iter().any(|(_, b)| b.is_shed()));
        let mut wanted = Vec::new();
        if needs_layout {
            let key = self.key;
            let raw = live_raw(app, live);
            let p = app.palette;
            prepare_memo(&mut app.preview.markdown.memo, key);
            let doc = app.editor.active().and_then(|d| d.path.clone());
            let remote = app.config.markdown_remote_images != Some(false);
            let image_path =
                |url: &str| crate::app::preview::markdown::image_path(doc.as_deref(), url, remote);
            let token = |kind| p.token(kind);
            let mut cx = Cx {
                frame,
                base: key.base,
                token: &token,
                surface: p.editor_bg,
                accent: p.accent,
                colors: Colors::new(p.text, p.text_muted, p.accent, p.dark),
                dark: p.dark,
                images: &app.preview.markdown.images,
                image_path: &image_path,
                wanted: Vec::new(),
                memo: &mut app.preview.markdown.memo,
                toggled: &app.preview.markdown.toggled,
                text: String::new(),
                chars: 0,
                raw: None,
                cell: None,
                preedit: false,
                line_numbers: key.numbers,
                tall: key.tall,
                repeat: 0,
                toggles: HashMap::new(),
                bleed: key.bleed,
            };
            let env = Env {
                color: cx.colors.text,
                depth: 0,
            };
            for range in &new_ranges {
                for i in range.clone() {
                    if !self.blocks[i].1.is_shed() {
                        continue;
                    }
                    let block = &parsed.blocks[i];
                    cx.repeat = if block_has_toggle(block) {
                        let h = parsed.hashes[i];
                        parsed.hashes[..i].iter().filter(|&&x| x == h).count() as u32
                    } else {
                        0
                    };
                    cx.toggles.clear();
                    let raw_i = raw.as_ref().filter(|r| r.0 == i);
                    let laid = layout_one_block(&mut cx, block, key.width, env, raw_i);
                    self.blocks[i].1 = Arc::new(laid);
                }
            }
            wanted = std::mem::take(&mut cx.wanted);
            // 只遍历新 `near` 范围内的活跃块清理缓存，不扫描全篇数万块。
            cx.memo.keep_only(
                new_ranges
                    .iter()
                    .flat_map(|r| self.blocks[r.clone()].iter().map(|(_, b)| &*b.memo)),
            );
        } else {
            app.preview.markdown.memo.keep_only(
                new_ranges
                    .iter()
                    .flat_map(|r| self.blocks[r.clone()].iter().map(|(_, b)| &*b.memo)),
            );
        }
        for (path, url) in wanted {
            app.request_markdown_image(&path, &url);
        }
        self.near = Some(near);
        self.caret.set(None);
    }

    /// 块数未变的常规单块实时输入快路径：仅重排改动的 1–2 个顶层块，就地平移后续块的 `tops`
    /// 与 `starts`，跳过全篇哈希表与块数组重建。不满足快路径前提时返回 `false` 交由
    /// [`super::lay_out`] 完整重建。
    pub(crate) fn patch_edited(
        &mut self,
        app: &mut App,
        frame: &mut Frame<'_>,
        parsed: &winc_preview::markdown::Markdown,
        source: usize,
        live: Option<&crate::app::preview::markdown::LiveLeaf>,
        near: Near,
    ) -> bool {
        let n = self.blocks.len();
        if n == 0 || n != parsed.blocks.len() || self.pending() {
            return false;
        }
        // 有收起的标题时交由完整排版处理隐藏块。
        if app
            .editor
            .active()
            .and_then(|d| app.preview.markdown.folds.get(&d.id))
            .is_some_and(|f| !f.is_empty())
        {
            return false;
        }

        // 找出所有需要重排的顶层块：旧光标块、新光标块，以及内容哈希发生变化的块。
        let old_live_block = self.live.as_ref().map(|k| k.0).filter(|&i| i < n);
        let new_live_block = live.map(|l| l.block).filter(|&i| i < n);
        let mut dirty: Vec<usize> = Vec::with_capacity(4);
        if let Some(b) = old_live_block {
            dirty.push(b);
        }
        if let Some(b) = new_live_block
            && !dirty.contains(&b)
        {
            dirty.push(b);
        }
        for i in 0..n {
            if self.blocks[i].0 != parsed.hashes[i] && !dirty.contains(&i) {
                dirty.push(i);
                if dirty.len() > 4 {
                    return false;
                }
            }
        }
        if dirty.is_empty() {
            return false;
        }
        dirty.sort_unstable();

        let key = self.key;
        for &i in &dirty {
            let block = &parsed.blocks[i];
            // 涉及折叠块、标题（影响 `sections` 目录）或块上下外边距变化时回退完整排版。
            if block_has_toggle(block)
                || matches!(block, winc_preview::markdown::Block::Heading(..))
                || self.sections.get(i).is_some_and(Option::is_some)
            {
                return false;
            }
            let (margin_top, margin_bottom) = margins(block, key.base, false);
            if i > 0 {
                let (_, prev_bottom) = margins(&parsed.blocks[i - 1], key.base, false);
                let expected =
                    self.tops[i - 1] + self.blocks[i - 1].1.height + prev_bottom.max(margin_top);
                if (self.tops[i] - expected).abs() > 0.01 {
                    return false;
                }
            }
            if i + 1 < n {
                let (next_top, _) = margins(&parsed.blocks[i + 1], key.base, false);
                let expected = self.tops[i] + self.blocks[i].1.height + margin_bottom.max(next_top);
                if (self.tops[i + 1] - expected).abs() > 0.01 {
                    return false;
                }
            }
        }

        let raw = live_raw(app, live);
        let live_key_now = live.map(|l| live_key(l, app.preedit_at(Focus::Editor)));
        let p = app.palette;
        prepare_memo(&mut app.preview.markdown.memo, key);
        let doc = app.editor.active().and_then(|d| d.path.clone());
        let remote = app.config.markdown_remote_images != Some(false);
        let image_path =
            |url: &str| crate::app::preview::markdown::image_path(doc.as_deref(), url, remote);
        let token = |kind| p.token(kind);

        let mut new_blocks: Vec<(usize, u64, LaidBlock)> = Vec::with_capacity(dirty.len());
        let wanted = {
            let mut cx = Cx {
                frame,
                base: key.base,
                token: &token,
                surface: p.editor_bg,
                accent: p.accent,
                colors: Colors::new(p.text, p.text_muted, p.accent, p.dark),
                dark: p.dark,
                images: &app.preview.markdown.images,
                image_path: &image_path,
                wanted: Vec::new(),
                memo: &mut app.preview.markdown.memo,
                toggled: &app.preview.markdown.toggled,
                text: String::new(),
                chars: 0,
                raw: None,
                cell: None,
                preedit: false,
                line_numbers: key.numbers,
                tall: key.tall,
                repeat: 0,
                toggles: HashMap::new(),
                bleed: key.bleed,
            };
            let env = Env {
                color: cx.colors.text,
                depth: 0,
            };
            for &i in &dirty {
                let block = &parsed.blocks[i];
                let base_hash = parsed.hashes[i];
                let raw_i = raw.as_ref().filter(|r| r.0 == i);
                let hash = match (raw_i, &live_key_now) {
                    (Some(_), Some(lk)) => winc_typeset::memo_key(|h| {
                        use std::hash::Hash;
                        (base_hash, lk).hash(h);
                    }),
                    _ => base_hash,
                };
                cx.repeat = 0;
                cx.toggles.clear();
                let laid = layout_one_block(&mut cx, block, key.width, env, raw_i);
                new_blocks.push((i, hash, laid));
            }
            std::mem::take(&mut cx.wanted)
        };

        // 就地替换改动的块并按高度差与字符数差平移后续块。
        for (i, hash, new_block) in new_blocks {
            let old_h = self.blocks[i].1.height;
            let old_chars = self.blocks[i].1.chars;
            let dy = new_block.height - old_h;
            let dc = new_block.chars as isize - old_chars as isize;
            let far = self
                .near
                .as_ref()
                .is_some_and(|kept| !kept.holds(i, self.tops[i], new_block.height));
            let is_live = new_live_block == Some(i);
            let stored = if far && !is_live {
                Arc::new(new_block.shed())
            } else {
                Arc::new(new_block)
            };
            self.blocks[i] = (hash, stored);
            if dy.abs() > 0.0 {
                for top in &mut self.tops[i + 1..] {
                    *top += dy;
                }
                self.height += dy;
            }
            if dc != 0 {
                for s in &mut self.starts[i + 1..] {
                    *s = usize::try_from(*s as isize + dc).unwrap_or(0);
                }
                self.chars = usize::try_from(self.chars as isize + dc).unwrap_or(0);
            }
        }

        // 就地更新原文范围与行号。
        self.ranges.clone_from(&parsed.ranges);
        if let Some(live) = live
            && let Some(start) = self.ranges.get(live.block).map(|r| r.start)
        {
            let len = app
                .editor
                .active()
                .and_then(|d| d.text().rope())
                .map(|r| r.len_bytes());
            let delta = len.map_or(0, |len| len as i64 - parsed.len as i64);
            if delta != 0 {
                let shift = |at: usize| usize::try_from(at as i64 + delta).unwrap_or(0);
                for range in self.ranges.iter_mut().filter(|r| r.start > start) {
                    *range = shift(range.start)..shift(range.end);
                }
            }
            self.ranges[live.block] = live.range.clone();
        }
        self.lines.clone_from(&parsed.lines);
        self.source = source;
        self.live = live_key_now;
        self.caret.set(None);

        for (path, url) in wanted {
            app.request_markdown_image(&path, &url);
        }

        if let Some(kept) = self.near.clone() {
            if !kept.covers(&near, app.preview.markdown.area.h) {
                self.refresh_near(app, frame, parsed, live, near);
            } else {
                let active = self.near_ranges(&kept);
                app.preview.markdown.memo.sweep(
                    active
                        .iter()
                        .flat_map(|r| self.blocks[r.clone()].iter().map(|(_, b)| &*b.memo)),
                );
            }
        } else {
            app.preview
                .markdown
                .memo
                .sweep(self.blocks.iter().map(|(_, b)| &*b.memo));
        }

        true
    }
}
