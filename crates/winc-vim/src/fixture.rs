//! 对照测试用例与录制结果的文件格式。
//!
//! 用例文件（`.cases`）手工编写，只有输入；录制文件（`.rec`）由 `cargo xtask vim-fixtures`
//! 生成，在每个用例之后记录 `vim` 执行的结果，文件开头记录 `vim` 的版本。两种文件的语法相同：
//!
//! ```text
//! # 注释
//! vim 9.1.1752
//! case delete-two-chars
//! text "abc def"
//! cursor 0 1
//! keys "a2x
//! > text "a def"
//! > cursor 0 1
//! > reg " v "bc"
//! > reg a v "bc"
//! ```

use std::fmt::Write as _;

/// 文本中的位置：行号与列号，都从 0 起；列号按字符（`char`）计。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// 行号。
    pub line: usize,
    /// 行内的字符偏移。
    pub col: usize,
}

/// 寄存器内容的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterKind {
    /// 按字符，记作 `v`。
    Charwise,
    /// 按行，记作 `V`。
    Linewise,
    /// 按块，记作 `b` 加块的宽度（显示列，取 `vim` 报告的值），例如 `b3`。
    Blockwise(usize),
}

/// 一个寄存器的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    /// 寄存器名。
    pub name: char,
    /// 内容的类型。
    pub kind: RegisterKind,
    /// 内容的各行，不含行尾的换行符。
    pub lines: Vec<String>,
}

/// 执行按键之后的状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    /// 文本的各行，不含换行符；至少一行。
    pub text: Vec<String>,
    /// 光标位置。
    pub cursor: Position,
    /// 非空的寄存器，按录制时查询的顺序排列。
    pub registers: Vec<Register>,
}

/// 一个用例：初始文本、光标、按键，以及录制的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Case {
    /// 名字，在文件内唯一，不含空白字符。
    pub name: String,
    /// 初始文本的各行，不含换行符；至少一行。
    pub text: Vec<String>,
    /// 初始的光标位置。
    pub cursor: Position,
    /// 在普通模式下依次输入的按键（按键记法）。
    pub keys: String,
    /// 录制的结果；用例文件中为 `None`。
    pub result: Option<State>,
}

/// 一个用例文件或录制文件的内容。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct File {
    /// 录制所用 `vim` 的版本，例如 `9.1.1752`；用例文件中为 `None`。
    pub vim: Option<String>,
    /// 全部用例，按文件中的顺序排列。
    pub cases: Vec<Case>,
}

/// 解析用例文件或录制文件。
///
/// # Errors
///
/// 语法错误、用例重名或用例缺少按键时返回说明，以「第 N 行」开头（行号从 1 起）。
pub fn parse(source: &str) -> Result<File, String> {
    let mut file = File::default();
    for (number, line) in source.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        parse_line(&mut file, line).map_err(|e| format!("第 {} 行：{e}", number + 1))?;
    }
    if let Some(case) = file.cases.iter().find(|c| c.keys.is_empty()) {
        return Err(format!("用例 {} 缺少 keys", case.name));
    }
    for case in &mut file.cases {
        if case.text.is_empty() {
            case.text.push(String::new());
        }
    }
    Ok(file)
}

/// 解析一行并记入 `file`。
fn parse_line(file: &mut File, line: &str) -> Result<(), String> {
    if line.trim().is_empty() || line.starts_with('#') {
        return Ok(());
    }
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    match word {
        "vim" if file.cases.is_empty() => file.vim = Some(rest.trim().to_owned()),
        "case" => {
            let name = rest.trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err("用例名不能为空，也不能含空白字符".into());
            }
            if file.cases.iter().any(|c| c.name == name) {
                return Err(format!("用例 {name} 重名"));
            }
            file.cases.push(Case {
                name: name.to_owned(),
                ..Case::default()
            });
        }
        "text" | "cursor" | "keys" | ">" => {
            let case = file.cases.last_mut().ok_or("缺少 case 行")?;
            parse_field(case, word, rest)?;
        }
        _ => return Err(format!("无法识别的行：{line}")),
    }
    Ok(())
}

/// 解析用例 `case` 之内以 `word` 开头的一行，`rest` 是该行余下的部分。
fn parse_field(case: &mut Case, word: &str, rest: &str) -> Result<(), String> {
    match (word, &mut case.result) {
        ("text", None) if case.keys.is_empty() => case.text.push(parse_string(rest)?.0),
        ("cursor", None) if case.keys.is_empty() => case.cursor = parse_position(rest)?,
        ("keys", None) if case.keys.is_empty() => {
            if rest.is_empty() || rest != rest.trim() {
                return Err("keys 不能为空，首尾的空格写作 <Space>".into());
            }
            rest.clone_into(&mut case.keys);
        }
        (">", result) if !case.keys.is_empty() => {
            let state = result.get_or_insert_with(State::default);
            let (word, rest) = rest.split_once(' ').unwrap_or((rest, ""));
            match word {
                "text" => state.text.push(parse_string(rest)?.0),
                "cursor" => state.cursor = parse_position(rest)?,
                "reg" => state.registers.push(parse_register(rest)?),
                _ => return Err(format!("无法识别的结果：{word}")),
            }
        }
        _ => {
            return Err(format!(
                "{word} 出现的位置不对：输入在 keys 之前，结果在 keys 之后"
            ));
        }
    }
    Ok(())
}

/// 解析 `行号 列号`。
fn parse_position(text: &str) -> Result<Position, String> {
    let mut numbers = text.split(' ').map(str::parse::<usize>);
    match (numbers.next(), numbers.next(), numbers.next()) {
        (Some(Ok(line)), Some(Ok(col)), None) => Ok(Position { line, col }),
        _ => Err(format!("位置应为「行号 列号」：{text}")),
    }
}

/// 解析 `名字 类型 "行"…`。
fn parse_register(text: &str) -> Result<Register, String> {
    let mut chars = text.chars();
    let (Some(name), Some(' ')) = (chars.next(), chars.next()) else {
        return Err(format!("寄存器应为「名字 类型 内容」：{text}"));
    };
    let (kind, mut rest) = chars
        .as_str()
        .split_once(' ')
        .unwrap_or((chars.as_str(), ""));
    let kind = match (kind, kind.strip_prefix('b').map(str::parse::<usize>)) {
        ("v", _) => RegisterKind::Charwise,
        ("V", _) => RegisterKind::Linewise,
        (_, Some(Ok(width))) => RegisterKind::Blockwise(width),
        _ => return Err(format!("寄存器类型应为 v、V 或 b 加宽度：{kind}")),
    };
    let mut lines = Vec::new();
    while !rest.is_empty() {
        let (line, tail) = parse_string(rest)?;
        lines.push(line);
        rest = tail.strip_prefix(' ').unwrap_or(tail);
    }
    Ok(Register { name, kind, lines })
}

/// 解析 `text` 开头的一个带引号的字符串，返回内容与余下的文本。
///
/// 转义：`\\`、`\"`、`\t`、`\r`、`\n`，以及 `\u{十六进制码位}`。
fn parse_string(text: &str) -> Result<(String, &str), String> {
    let invalid = || format!("字符串不完整或转义有误：{text}");
    let mut chars = text.strip_prefix('"').ok_or_else(invalid)?.chars();
    let mut value = String::new();
    loop {
        match chars.next().ok_or_else(invalid)? {
            '"' => return Ok((value, chars.as_str())),
            '\\' => value.push(match chars.next().ok_or_else(invalid)? {
                '\\' => '\\',
                '"' => '"',
                't' => '\t',
                'r' => '\r',
                'n' => '\n',
                'u' => {
                    let rest = chars.as_str().strip_prefix('{').ok_or_else(invalid)?;
                    let (digits, tail) = rest.split_once('}').ok_or_else(invalid)?;
                    chars = tail.chars();
                    u32::from_str_radix(digits, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(invalid)?
                }
                _ => return Err(invalid()),
            }),
            c => value.push(c),
        }
    }
}

/// 把 `file` 写成文本；[`parse`] 该文本得到与 `file` 相等的值。
///
/// `file` 中各用例的文本至少有一行、名字与按键符合 [`Case`] 的约定时成立。
pub fn write(file: &File) -> String {
    let mut out = String::new();
    // 向 `String` 写入不会失败。
    let _ = write_file(&mut out, file);
    out
}

/// 把 `file` 写入 `out`。
fn write_file(out: &mut String, file: &File) -> std::fmt::Result {
    if let Some(vim) = &file.vim {
        writeln!(out, "vim {vim}")?;
    }
    for case in &file.cases {
        writeln!(out, "\ncase {}", case.name)?;
        write_state(out, "", &case.text, case.cursor, &[])?;
        writeln!(out, "keys {}", case.keys)?;
        if let Some(state) = &case.result {
            write_state(out, "> ", &state.text, state.cursor, &state.registers)?;
        }
    }
    Ok(())
}

/// 以 `prefix` 为每行的前缀写出文本、光标与寄存器。
fn write_state(
    out: &mut String,
    prefix: &str,
    text: &[String],
    cursor: Position,
    registers: &[Register],
) -> std::fmt::Result {
    for line in text {
        writeln!(out, "{prefix}text {}", quoted(line))?;
    }
    writeln!(out, "{prefix}cursor {} {}", cursor.line, cursor.col)?;
    for register in registers {
        write!(out, "{prefix}reg {} ", register.name)?;
        match register.kind {
            RegisterKind::Charwise => out.push('v'),
            RegisterKind::Linewise => out.push('V'),
            RegisterKind::Blockwise(width) => write!(out, "b{width}")?,
        }
        for line in &register.lines {
            write!(out, " {}", quoted(line))?;
        }
        out.push('\n');
    }
    Ok(())
}

/// 返回带引号的 `text`，引号、反斜杠与控制字符写成转义。
fn quoted(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;
