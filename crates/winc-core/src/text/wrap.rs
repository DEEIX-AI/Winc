//! 自动换行：按显示宽度把一行断成若干段，即屏幕上的若干显示行。
//!
//! 断点优先选在空白或标点之后；中日韩文字之间的任意位置都可以断开；单词比整行还长时在行宽
//! 处硬断。行首禁则的标点（如 `，。）」`）不放到下一段开头，行尾禁则的开括号（如 `（「`）不放
//! 在上一段末尾。后续各段与第一段的缩进对齐；缩进超过行宽的一半时不再缩进。

use ropey::RopeSlice;

use crate::text::char_cells;

/// 一行断开后的各段起止：后续各段的起始位置，以及后续各段的缩进宽度。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breaks {
    /// 第二段起各段的开头（行内的字符序号，从 0 起），严格递增。
    pub starts: Vec<usize>,
    /// 后续各段的缩进宽度（显示列）；第一段为 0。
    pub indent: usize,
}

impl Breaks {
    /// 返回断开后的段数，即这一行占用的显示行数。
    pub fn rows(&self) -> usize {
        self.starts.len() + 1
    }

    /// 返回第 `col` 个字符所在的段号（从 0 起）。
    ///
    /// 分界处的字符属于后一段：光标位于分界处时绘制在下一行的开头。
    pub fn row_of(&self, col: usize) -> usize {
        self.starts.partition_point(|&s| s <= col)
    }

    /// 返回第 `row` 段的字符范围（行内字符序号，左闭右开）；`len` 是整行的字符数。
    ///
    /// # Panics
    ///
    /// `row` 不小于 [`Breaks::rows`] 时 panic。
    pub fn range(&self, row: usize, len: usize) -> std::ops::Range<usize> {
        let end = self.starts.get(row).copied().unwrap_or(len);
        self.start_of(row)..end
    }

    /// 返回第 `row` 段开头的字符序号（行内，从 0 起）；第一段为 0。
    ///
    /// # Panics
    ///
    /// `row` 不小于 [`Breaks::rows`] 时 panic。
    pub fn start_of(&self, row: usize) -> usize {
        if row == 0 { 0 } else { self.starts[row - 1] }
    }

    /// 返回第 `row` 段开头的缩进宽度（显示列）；第一段为 0。
    pub fn indent_of(&self, row: usize) -> usize {
        if row == 0 { 0 } else { self.indent }
    }
}

/// 后续各段缩进上限的分母：缩进宽度超过行宽的 `1 / MAX_INDENT_SHARE` 时不再缩进。
///
/// 取 2（经验值），即缩进最多占行宽的一半。
const MAX_INDENT_SHARE: usize = 2;

/// 按 `width` 个显示列把 `chars` 断开，返回分段结果。
///
/// `chars` 是这一行的字符（不含换行符）；`width` 小于 1 时按 1 处理；`tab_width` 是制表位宽度
/// （显示列）。
///
/// # Panics
///
/// `chars` 含制表符且 `tab_width` 为 0 时 panic。
pub fn breaks(chars: &[char], width: usize, tab_width: usize) -> Breaks {
    let width = width.max(1);
    // 整行放得下时无需分段，返回 `Breaks::default()`（缩进也记为 0）。调用方为了不必取出字符表，
    // 通常已先量过行宽，这里只量到超出行宽为止，代价不超过一段的宽度。保留这次测量，使本函数对
    // 任何输入都给出相同的结果。
    let mut total = 0;
    for &c in chars {
        total += char_cells(c, total, tab_width);
        if total > width {
            break;
        }
    }
    if total <= width {
        return Breaks::default();
    }

    let lead = chars.iter().take_while(|&&c| c == ' ' || c == '\t').count();
    let lead_cells = chars[..lead]
        .iter()
        .fold(0, |cell, &c| cell + char_cells(c, cell, tab_width));
    let indent = if lead_cells * MAX_INDENT_SHARE <= width {
        lead_cells
    } else {
        0
    };

    let mut out = Breaks {
        starts: Vec::new(),
        indent,
    };
    let mut start = 0;
    while start < chars.len() {
        // `offset` 是本段起始的显示列，后续各段先留出缩进。向后量到放不下的字符为止，`fits` 记录
        // 放得下的最后位置；回退查找断点时从 `fits` 开始，无需重新计算宽度。
        let offset = if out.starts.is_empty() { 0 } else { indent };
        let mut cell = offset;
        let mut end = start;
        let mut fits = start;
        while end < chars.len() {
            let w = char_cells(chars[end], cell, tab_width);
            // 每段至少放一个字符，否则行宽小于该字符的宽度时 `start` 无法前进。
            if cell + w > width && end > start {
                break;
            }
            cell += w;
            end += 1;
            if cell <= width {
                fits = end;
            }
        }
        if end >= chars.len() {
            break;
        }
        // 放不下的字符是空白时，断在这串空白之后：空白留在上一段末尾，超出部分不可见，下一段从
        // 非空白字符开始。
        let cut = if matches!(chars[end], ' ' | '\t') {
            let mut cut = end;
            while cut < chars.len() && matches!(chars[cut], ' ' | '\t') {
                cut += 1;
            }
            cut
        } else {
            // 从 `fits` 向前查找可断开的位置，找不到时在放不下的字符处硬断。`earliest` 是最靠前的
            // 断点：第一段不能断在缩进内，否则第一段只剩缩进，浪费一行。
            let earliest = if start == 0 {
                (lead + 1).min(end)
            } else {
                start + 1
            };
            let mut cut = fits.clamp(earliest, end);
            while cut > earliest && !can_break(chars[cut - 1], chars[cut]) {
                cut -= 1;
            }
            if can_break(chars[cut - 1], chars[cut]) {
                cut
            } else {
                end
            }
        };
        if cut >= chars.len() {
            break;
        }
        out.starts.push(cut);
        start = cut;
    }
    out
}

/// 是否可以在相邻字符 `a` 和 `b` 之间断开。
fn can_break(a: char, b: char) -> bool {
    if no_line_start(b) || no_line_end(a) {
        return false;
    }
    if a == ' ' || a == '\t' {
        return b != ' ' && b != '\t';
    }
    if is_wide(a) || is_wide(b) {
        return true;
    }
    // 代码中的标点在其后断开（如 `a,`、`x-`、`a/`），且下一个字符不是 ASCII 标点。
    matches!(
        a,
        '-' | '/' | ',' | ';' | ')' | ']' | '}' | '>' | '|' | '&' | '?' | '!' | ':'
    ) && !b.is_ascii_punctuation()
}

/// 是否为中日韩文字或全角标点；这些字符的两侧都可以断开。
///
/// 按 Unicode 码点区间判断：谚文字母与音节、CJK 符号与标点、假名、汉字、全角形式等。
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// 是否为不能放在一行开头的字符（行首禁则），即不能断在该字符之前。
///
/// 依据中文排版的避头尾规则：收尾标点、后引号、后括号不能出现在行首；小假名同理。
fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '）'
            | '】'
            | '」'
            | '』'
            | '》'
            | '〉'
            | '”'
            | '’'
            | '…'
            | '·'
            | '～'
            | '%'
            | '‰'
            | ')'
            | ']'
            | '}'
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | 'ー'
            | 'ゝ'
            | 'ゞ'
            | '々'
    ) || ('ぁ'..='ゖ').contains(&c) && "ぁぃぅぇぉっゃゅょゎゕゖ".contains(c)
        || ('ァ'..='ヶ').contains(&c) && "ァィゥェォッャュョヮヵヶ".contains(c)
}

/// 是否为不能放在一行末尾的字符（行尾禁则），即不能断在该字符之后。
///
/// 依据中文排版的避头尾规则：开引号、开括号不能留在行末。
fn no_line_end(c: char) -> bool {
    matches!(
        c,
        '（' | '【' | '「' | '『' | '《' | '〈' | '“' | '‘' | '(' | '[' | '{' | '$' | '￥' | '#'
    )
}

/// 返回 `chars` 中第 `col` 个字符在其所在段内的起始显示列（含该段的缩进宽度）。
///
/// `chars` 是整行的内容，`breaks` 是对该行调用 [`breaks`] 的结果。
pub fn cell_in_row(chars: RopeSlice<'_>, breaks: &Breaks, col: usize, tab_width: usize) -> usize {
    let row = breaks.row_of(col);
    let start = breaks.start_of(row);
    let mut cell = breaks.indent_of(row);
    // 从段首开始量，不从行首跳过前面各段的字符：长行的后面几段每帧都要量。
    for c in chars.chars_at(start).take(col - start) {
        cell += char_cells(c, cell, tab_width);
    }
    cell
}

#[cfg(test)]
mod tests;
