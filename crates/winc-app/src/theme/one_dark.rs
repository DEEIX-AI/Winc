//! One Dark：沿用 Atom One Dark 的经典底色与语法色，适配 Winc 的界面控件和终端。

use super::palettes::hex;
use super::{Color, Palette, Tokens, shade};

const BACKGROUND: Color = hex(0x282C34);
const FOREGROUND: Color = hex(0xABB2BF);
const BLUE: Color = hex(0x61AFEF);
const RED: Color = hex(0xE06C75);
const GREEN: Color = hex(0x98C379);
const YELLOW: Color = hex(0xE5C07B);
const ORANGE: Color = hex(0xD19A66);
const MAGENTA: Color = hex(0xC678DD);
const CYAN: Color = hex(0x56B6C2);
const CURSOR: Color = hex(0x528BFF);
const HIGHLIGHT: Color = hex(0x2C313A);

/// 蓝灰底、亮蓝光标；注释和行号比原版调亮，避免在深色底上太淡。
pub(super) static ONE_DARK: Palette = Palette {
    caret: CURSOR,
    current_line: HIGHLIGHT,
    diff_split_modified: true,
    diff_modified: shade(hex(0x75B98E), BACKGROUND, true),
    diff_deleted: shade(RED, BACKGROUND, true),
    ..Palette::build(Tokens {
        dark: true,
        frame: hex(0x21252B),
        card: BACKGROUND,
        popup: HIGHLIGHT,
        text: FOREGROUND,
        text_muted: hex(0x929BAB),
        text_dim: hex(0x737D8F),
        accent: CURSOR,
        accent_fill: hex(0x3A63AE),
        selection: hex(0x3E4451),
        gutter: [hex(0x737D8F), FOREGROUND],
        error: RED,
        warning: YELLOW,
        git: [BLUE, GREEN, RED, ORANGE],
        tinted_tab: true,
        syntax: [
            FOREGROUND,
            MAGENTA,
            YELLOW,
            GREEN,
            ORANGE,
            hex(0x8993A5),
            BLUE,
            CYAN,
            RED,
        ],
        ansi: [
            hex(0x21252B),
            RED,
            GREEN,
            YELLOW,
            BLUE,
            MAGENTA,
            CYAN,
            FOREGROUND,
            hex(0x838C9D),
            hex(0xF0848C),
            hex(0xB0D58C),
            hex(0xF3CE8C),
            hex(0x85C1F6),
            hex(0xD994E8),
            hex(0x77CAD4),
            hex(0xDCDFE4),
        ],
    })
};
