//! Markdown 写作命令的图标：格式工具栏与 `/` 命令列表使用，线条风格同其他图标，约 12×12，
//! 以中心点定位。

use super::*;

/// Markdown 写作命令的图形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritingGlyph {
    Bold,
    Italic,
    Strike,
    Code,
    Highlight,
    Link,
    /// 标题，带级别（1–3）。
    Heading(u8),
    Bullet,
    Ordered,
    Task,
    Quote,
    Callout,
    Details,
    CodeBlock,
    Math,
    InlineMath,
    Diagram,
    Table,
    Divider,
    Image,
    Contents,
    Date,
}

/// 绘制写作命令的图标 `glyph`，中心在 `(cx, cy)`。
pub fn writing(
    frame: &mut Frame<'_>,
    center: (f32, f32),
    glyph: WritingGlyph,
    color: Color,
    clip: Rect,
) {
    let icon = match glyph {
        WritingGlyph::Bold => Icon::Bold,
        WritingGlyph::Italic => Icon::Italic,
        WritingGlyph::Strike => Icon::Strikethrough,
        WritingGlyph::Code | WritingGlyph::CodeBlock => Icon::Code,
        WritingGlyph::Highlight => Icon::Highlight,
        WritingGlyph::Link => Icon::Link,
        WritingGlyph::Heading(0 | 1) => Icon::Heading1,
        WritingGlyph::Heading(2) => Icon::Heading2,
        // 图标集只到三级标题，更深的级别共用三级的图标。
        WritingGlyph::Heading(_) => Icon::Heading3,
        WritingGlyph::Bullet => Icon::List,
        WritingGlyph::Ordered => Icon::ListOrdered,
        WritingGlyph::Task => Icon::SquareCheck,
        WritingGlyph::Quote => Icon::Quote,
        WritingGlyph::Callout => Icon::Callout,
        WritingGlyph::Details => Icon::Details,
        WritingGlyph::Math => Icon::Sigma,
        WritingGlyph::InlineMath => Icon::InlineMath,
        WritingGlyph::Diagram => Icon::Diagram,
        WritingGlyph::Table => Icon::Table,
        WritingGlyph::Divider => Icon::Divider,
        WritingGlyph::Image => Icon::Image,
        WritingGlyph::Contents => Icon::TableOfContents,
        WritingGlyph::Date => Icon::Calendar,
    };
    // 与文件图标、列表视图共用的图形按各自的设计稿对齐，其余约 12 逻辑像素见方。
    let extent = match glyph {
        WritingGlyph::Bullet => 13.5,
        WritingGlyph::Image => 12.5,
        WritingGlyph::Table => 12.9,
        _ => 12.0,
    };
    draw(frame, icon, center, extent, STROKE, color, clip);
}
