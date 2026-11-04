//! Winc 系列主题：为长时间写代码设计的两深两浅四套配色，也是默认的主题（深色「Winc 夜航」、
//! 浅色「Winc 晴昼」）。
//!
//! 取色按可测量的对比度约束：底色不用纯黑纯白；正文至少 7:1（WCAG AAA），次要文字与注释至少
//! 4.5:1，其余语法色至少 6:1，强调色底上的文字至少 4.5:1。语法色按 `OKLCH` 取相同的明度和彩度，
//! 只在色相上拉开距离，色相感知较弱的用户仍能靠明度区分；Git 新增用偏青的绿、删除用偏橙的红，
//! 冲突用品红。

use super::*;

const fn hex(rgb: u32) -> Color {
    Color::hex(rgb)
}

/// 「Winc 夜航」：默认的深色主题，蓝灰炭底。
///
/// 橙色关键字、浅蓝函数、青色类型、海绿字符串、兰紫数字、浅金装饰器、粉色属性名；底色不用纯黑，
/// 避免长时间阅读时纯黑底与亮字之间的强对比。
pub static WINC_NIGHT: Palette = Palette::build(Tokens {
    dark: true,
    frame: hex(0x15171C),
    card: hex(0x1B1E24),
    popup: hex(0x23272E),
    text: hex(0xD4D8DF),
    text_muted: hex(0x959CA8),
    text_dim: hex(0x656C78),
    accent: hex(0x78AEF2),
    accent_fill: hex(0x3D6FC4),
    selection: hex(0x2C4468),
    gutter: [hex(0x4E5562), hex(0xA9B0BC)],
    error: hex(0xF07F7F),
    warning: hex(0xE2BA6A),
    git: [hex(0x72A7EC), hex(0x5FBFA0), hex(0xEB7F6C), hex(0xD898E0)],
    tinted_tab: true,
    syntax: [
        hex(0xD4D8DF),
        hex(0xD7915C),
        hex(0x6DDAD6),
        hex(0x75B18B),
        hex(0xC28EC4),
        hex(0x7F8796),
        hex(0x6FB7F0),
        hex(0xE4CF7A),
        hex(0xF9AEB9),
    ],
    ansi: [
        hex(0x2A2E36),
        hex(0xEC7A7A),
        hex(0x8EC48A),
        hex(0xDDBB6E),
        hex(0x6FA6EE),
        hex(0xC890D0),
        hex(0x5FC6C2),
        hex(0xC9CED6),
        hex(0x6B7280),
        hex(0xF59A98),
        hex(0xA6D6A0),
        hex(0xEBCD88),
        hex(0x91BCF5),
        hex(0xDAAAE0),
        hex(0x80DAD6),
        hex(0xEEF0F4),
    ],
});

/// 「Winc 远山」：「Winc 夜航」的柔和版，底色亮一档的岩灰、正文稍暗，明暗反差小，适合白天开灯的
/// 房间；色相和「Winc 夜航」一一对应。
pub static WINC_SLATE: Palette = Palette::build(Tokens {
    dark: true,
    frame: hex(0x1E2127),
    card: hex(0x252930),
    popup: hex(0x2C3038),
    text: hex(0xCED3DB),
    text_muted: hex(0x99A0AC),
    text_dim: hex(0x6C7380),
    accent: hex(0x7DB2F2),
    accent_fill: hex(0x4472C2),
    selection: hex(0x35496B),
    gutter: [hex(0x575E6B), hex(0xADB4BF)],
    error: hex(0xF08585),
    warning: hex(0xE2BD70),
    git: [hex(0x78ABEC), hex(0x66C2A5), hex(0xEC8573), hex(0xDA9DE2)],
    tinted_tab: true,
    syntax: [
        hex(0xCED3DB),
        hex(0xDB9763),
        hex(0x72DCD8),
        hex(0x7DB892),
        hex(0xC895CA),
        hex(0x8A92A0),
        hex(0x76BCF2),
        hex(0xE6D282),
        hex(0xF9B3BE),
    ],
    ansi: [
        hex(0x343943),
        hex(0xEE8080),
        hex(0x93C88F),
        hex(0xDFBF74),
        hex(0x76ABEE),
        hex(0xCC96D4),
        hex(0x66CAC6),
        hex(0xCED3DB),
        hex(0x767D8A),
        hex(0xF6A09E),
        hex(0xABD9A5),
        hex(0xECD08E),
        hex(0x97C0F5),
        hex(0xDCAFE2),
        hex(0x86DCD8),
        hex(0xF0F2F5),
    ],
});

/// 「Winc 晴昼」：默认的浅色主题，略带灰的白底。
///
/// 砖红关键字、蓝色函数、钢青类型、深绿字符串、靛紫数字、橄榄装饰器、梅红属性名；底色不用纯白，
/// 以降低屏幕亮度高时的眩光。
pub static WINC_DAY: Palette = Palette::build(Tokens {
    dark: false,
    frame: hex(0xE9EBEF),
    card: hex(0xFBFBFC),
    popup: hex(0xFFFFFF),
    text: hex(0x24272E),
    text_muted: hex(0x5C6370),
    text_dim: hex(0x9AA0AB),
    accent: hex(0x2B66D2),
    accent_fill: hex(0x2B66D2),
    selection: hex(0xC9DCFA),
    gutter: [hex(0xA4AAB5), hex(0x5A606B)],
    error: hex(0xC2373A),
    warning: hex(0x9A6200),
    git: [hex(0x2B66D2), hex(0x16805F), hex(0xC4432F), hex(0xA43F98)],
    tinted_tab: true,
    syntax: [
        hex(0x24272E),
        hex(0xA43E3A),
        hex(0x156686),
        hex(0x1D691A),
        hex(0x58309D),
        hex(0x6B717D),
        hex(0x0A59CA),
        hex(0x574504),
        hex(0x7E2166),
    ],
    ansi: [
        hex(0x24272E),
        hex(0xC23B3B),
        hex(0x2E7D32),
        hex(0x8A6400),
        hex(0x2563C9),
        hex(0x8E3FB0),
        hex(0x0F7480),
        hex(0x6B7280),
        hex(0x575E6A),
        hex(0xD14B4B),
        hex(0x3C8F3F),
        hex(0x9E7400),
        hex(0x3577E0),
        hex(0xA052C4),
        hex(0x1A8794),
        hex(0xB4B9C2),
    ],
});

/// 「Winc 宣纸」：「Winc 晴昼」的暖色版，米白的纸底、暖灰的字。
///
/// 色相与「Winc 晴昼」一一对应，再按纸底略微压暗，使在暖色底上的对比度与「Winc 晴昼」一致。
pub static WINC_PAPER: Palette = Palette::build(Tokens {
    dark: false,
    frame: hex(0xEAE6DE),
    card: hex(0xF7F4EE),
    popup: hex(0xFCFAF6),
    text: hex(0x2A2824),
    text_muted: hex(0x5F5A52),
    text_dim: hex(0x9C968C),
    accent: hex(0x2A62C8),
    accent_fill: hex(0x2A62C8),
    selection: hex(0xD6E0EE),
    gutter: [hex(0xA9A398), hex(0x5F5A52)],
    error: hex(0xBB3434),
    warning: hex(0x935C00),
    git: [hex(0x2A62C8), hex(0x157A5A), hex(0xBC402C), hex(0x9D3B92)],
    tinted_tab: true,
    syntax: [
        hex(0x2A2824),
        hex(0x9C3934),
        hex(0x12607E),
        hex(0x1B6318),
        hex(0x542D96),
        hex(0x6A665E),
        hex(0x0954C0),
        hex(0x524000),
        hex(0x781E5F),
    ],
    ansi: [
        hex(0x2A2824),
        hex(0xBA3737),
        hex(0x2C772F),
        hex(0x846000),
        hex(0x245EC0),
        hex(0x8A3CA9),
        hex(0x0E6E7A),
        hex(0x67625A),
        hex(0x55504A),
        hex(0xC94646),
        hex(0x388839),
        hex(0x966E00),
        hex(0x3370D6),
        hex(0x9A4FBC),
        hex(0x18818D),
        hex(0xB3ADA2),
    ],
});
