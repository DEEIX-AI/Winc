//! Markdown 预览每一帧的排版决策：命中复用、跨出 [`Near`] 就地刷新、单块实时输入修补与完整重排。

use std::sync::Arc;

use winc_render::{Frame, Rect};

use crate::app::App;
use crate::app::preview::markdown::LiveLeaf;
use crate::app::preview::markdown::laid::*;
use crate::layout::drawn_scroll;

use super::room_below;

/// 本帧排版决策的输入条件。
pub(super) struct LayoutInput<'a> {
    pub(super) parsed: &'a Arc<winc_preview::markdown::Markdown>,
    pub(super) key: Key,
    pub(super) source: usize,
    pub(super) live: Option<&'a LiveLeaf>,
    pub(super) live_key: Option<&'a LiveKey>,
    pub(super) near: Near,
    pub(super) area: Rect,
    pub(super) pad: f32,
    pub(super) loaded: Vec<std::path::PathBuf>,
    pub(super) toggled: Option<String>,
    pub(super) refold: bool,
}

/// 本帧排版结果以及需要保留的光标屏幕高度与块滑动动画快照。
pub(super) struct LayoutOutput {
    pub(super) laid: Laid,
    pub(super) still: Option<(usize, f32)>,
    pub(super) before: Option<(Before, bool)>,
}

/// 按当前状态复用、增量刷新、单块就地修补或完整重排整篇 Markdown 文档。
pub(super) fn resolve_layout(
    app: &mut App,
    frame: &mut Frame<'_>,
    input: LayoutInput<'_>,
) -> LayoutOutput {
    let LayoutInput {
        parsed,
        key,
        source,
        live,
        live_key,
        near,
        area,
        pad,
        loaded,
        toggled,
        refold,
    } = input;
    let mut still = None;
    let mut before = None;
    let laid = match app.preview.markdown.layout.take() {
        Some(laid)
            if laid.key == key
                && laid.source == source
                && laid.live.as_ref() == live_key
                && loaded.is_empty()
                && toggled.is_none()
                && !refold
                && !laid.pending()
                && (laid.near.as_ref()).is_none_or(|kept| kept.covers(&near, area.h)) =>
        {
            laid
        }
        Some(mut laid)
            if laid.key == key
                && laid.source == source
                && laid.live.as_ref() == live_key
                && loaded.is_empty()
                && toggled.is_none()
                && !refold
                && !laid.pending() =>
        {
            // 纯滚动或跳转跨出 `Near` 缓冲：就地释放离屏块、重排新进入视口的块，不重建整篇排版。
            laid.refresh_near(app, frame, parsed, live, near);
            laid
        }
        mut old => {
            let hscroll = old.as_ref().map(|l| l.hscroll.clone()).unwrap_or_default();
            let followed = old
                .as_ref()
                .filter(|l| l.source == source && l.live.as_ref() == live_key)
                .and_then(|l| l.followed);
            // 继续上一帧未完成的排版，或视口移到了保留的范围之外：排版条件和文字都未变，选区、
            // 查找结果和光标位置保留。
            let resumed = old.as_ref().is_some_and(|l| {
                l.key == key
                    && l.source == source
                    && l.live.as_ref() == live_key
                    && loaded.is_empty()
                    && toggled.is_none()
                    && !refold
            });
            // 文字变了：选区、查找用的文字作废。只是重排（宽度、图片变了）时选区留着。
            if old.as_ref().is_none_or(|l| l.source != source) {
                app.preview.markdown.selection = None;
            }
            let current_rev = app.editor.active().map(|d| d.revision);
            let mut slide = false;
            if !resumed {
                app.preview.markdown.find_doc = None;
                let clicked = app.preview.markdown.clicked.take();
                still = old_caret(app, old.as_ref(), source, live_key, clicked);
                slide = std::mem::take(&mut app.preview.markdown.slide);
                // 常规文字输入（此前已处于编辑态、文档版本号已变且未要求滑动或折叠）无需过渡动画，
                // 直接清除旧过渡并跳过全篇快照克隆。
                let typing = !slide
                    && !refold
                    && old
                        .as_ref()
                        .is_some_and(|l| l.live.as_ref().is_some_and(|k| Some(k.2) != current_rev));
                if typing {
                    app.preview.markdown.motion = None;
                } else {
                    // 收起、展开标题时各块同样滑到新位置，不只在可编辑的预览中。
                    before = old
                        .as_ref()
                        .filter(|l| l.key == key && (app.markdown_live() || refold))
                        .map(|l| (Before::of(app, l), slide));
                }
            }
            // 单块常规实时输入快路径：条件与块数未变且无需过渡动画时，就地重排改动的块。
            let patched = if before.is_none()
                && !slide
                && !resumed
                && loaded.is_empty()
                && toggled.is_none()
                && !refold
                && old.as_ref().is_some_and(|l| {
                    l.key == key && !l.pending() && l.blocks.len() == parsed.blocks.len()
                }) {
                let mut candidate = old.take().unwrap();
                if candidate.patch_edited(app, frame, parsed, source, live, near.clone()) {
                    candidate.hscroll = hscroll.clone();
                    candidate.followed = followed;
                    Some(candidate)
                } else {
                    old = Some(candidate);
                    None
                }
            } else {
                None
            };
            if let Some(laid) = patched {
                laid
            } else {
                // 文字未变时保留旧的各块供沿用，并记录视口顶边所在的块，排版期间保持其屏幕位置。
                let same = (old.as_ref())
                    .filter(|l| l.source == source && l.blocks.len() == parsed.blocks.len());
                let top = drawn_scroll(app.preview.markdown.scroll());
                // 顶边在该块里所占的比例：块的高度变了，顶边仍在同一块的相同比例处。
                let held = same.and_then(|l| {
                    let (i, off) = l.place_at(top - l.pad)?;
                    Some((i, off / room_below(l, i).max(1.0)))
                });
                let mut window =
                    same.map(|l| l.blocks_in(top - l.pad - area.h, top + area.h * 2.0));
                let mut kept = same.map(|l| l.blocks.clone());
                let was_pending = same.is_some_and(Laid::pending);
                // 有图片刚读好时，用到它的块大小变了，需要重排；折叠块开合时，它所在的块需要
                // 重排；其余块沿用上次的结果。
                let mut reuse = old.filter(|l| l.key == key).map(|l| {
                    l.fresh()
                        .into_iter()
                        .filter(|(_, block)| {
                            !block.uses_any(&loaded)
                                && toggled.as_deref().is_none_or(|url| !block.links_to(url))
                        })
                        .collect()
                });
                let deadline = app.preview.markdown.catch_up.deadline();
                // 视口上方的块高度变化后，视口内的块可能与排版前估计的不同：仍有未排的块时再排一轮。
                let mut laid = loop {
                    let later = (kept.as_deref().zip(window.clone())).map(|(old, window)| Later {
                        old,
                        window,
                        deadline,
                    });
                    let fresh = reuse.take().unwrap_or_default();
                    let laid = lay_out(
                        app,
                        frame,
                        parsed,
                        key,
                        source,
                        live,
                        fresh,
                        later,
                        Some(near.clone()),
                    );
                    let y = held.map_or(top - pad, |(i, share)| {
                        laid.tops[i] + share.min(1.0) * room_below(&laid, i)
                    });
                    let seen = laid.blocks_in(y, y + area.h);
                    let waits = |i: usize| laid.stale.get(i).copied().unwrap_or(false);
                    if window.as_ref() == Some(&seen) || !seen.clone().any(waits) {
                        break laid;
                    }
                    (reuse, window) = (Some(laid.fresh()), Some(seen));
                    kept = Some(laid.blocks);
                };
                // 分帧排版、图片加载完成、收起或展开标题后，视口顶边所在块的屏幕位置不变。
                let anchored = was_pending || laid.pending() || !loaded.is_empty() || refold;
                if let Some((i, share)) = held.filter(|_| anchored) {
                    let y = laid.tops[i] + share.min(1.0) * room_below(&laid, i) + pad;
                    app.preview.markdown.set_scroll(y);
                    still = None;
                }
                app.preview.markdown.catch_up.pending = laid.pending();
                // 释放了要画的项的块不再使用排版缓存，缓存只保留其余的块用到的条目。
                if laid.near.is_some() {
                    let used = laid.blocks.iter().map(|(_, block)| &*block.memo);
                    app.preview.markdown.memo.keep_only(used);
                }
                laid.hscroll = hscroll;
                laid.followed = followed;
                laid
            }
        }
    };
    LayoutOutput {
        laid,
        still,
        before,
    }
}
