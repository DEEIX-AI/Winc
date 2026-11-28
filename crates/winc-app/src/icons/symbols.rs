//! 大纲中各类符号的图标：线条风格，与文件树的文件图标同宽、同笔宽，按种类着色。

use super::*;

/// 符号图标传给 [`draw`] 的外形尺寸（逻辑像素）。文件图标里与符号同形的记号（JSON 的花括号、包的
/// 立方体）也用它，两处画出来一样大。
pub(super) const SYMBOL: f32 = 11.0;

/// 符号图标的图形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolGlyph {
    /// 函数、方法：一个立方体。
    Cube,
    /// 类型（类、结构体、接口……）：一张卡片，上沿下面一道横线隔出表头。
    Card,
    /// 模块、命名空间：一对花括号。
    Braces,
    /// 常量、变量：一个圆，中间一点。
    Field,
    /// Markdown 标题：一段文字，三行横线，最后一行短。
    Text,
    /// 配置文件的小节：一对方括号。
    Brackets,
}

/// 绘制符号图标：左上角在 `(x, y)`，占 `FILE_WIDTH` × 15 的位置，与 [`file()`] 相同，文件树和
/// 大纲里同一列的图标上下、左右都对齐；图形约 11 逻辑像素见方，与文件图标纸面上的记号同大。
pub fn symbol(
    frame: &mut Frame<'_>,
    (x, y): (f32, f32),
    glyph: SymbolGlyph,
    color: Color,
    clip: Rect,
) {
    let icon = match glyph {
        SymbolGlyph::Cube => Icon::Cube,
        SymbolGlyph::Card => Icon::Card,
        SymbolGlyph::Braces => Icon::Braces,
        SymbolGlyph::Field => Icon::SymbolField,
        // 标题文字前已写出 `#`，图标不再重复井号，只表示这是一段文字。
        SymbolGlyph::Text => Icon::Text,
        SymbolGlyph::Brackets => Icon::Brackets,
    };
    let center = (x + FILE_WIDTH / 2.0, y + 7.5);
    draw(frame, icon, center, SYMBOL, STROKE, color, clip);
}
