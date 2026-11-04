//! 格式化命令的拆分和占位符展开：命令不经过 shell，按空白拆成参数。

use std::ffi::OsString;
use std::path::Path;

use winc_i18n::t;

/// 占位符：被格式化文件的完整路径。
pub(super) const FILE: &str = "{file}";
/// 占位符：文件所属 crate 的 Rust 版次（`edition`）。
pub(super) const RUST_EDITION: &str = "{rust-edition}";

/// 把命令按空白拆成参数列表，不经过 shell。
///
/// 单引号内的字符原样保留；双引号内 `\"` 和 `\\` 是转义；引号之外、位于参数开头的 `~/`
/// 替换为 `home`（`home` 为 `None` 时保留 `~`）。
///
/// # Errors
///
/// 单引号或双引号没有配对时返回错误说明。
pub(super) fn split(command: &str, home: Option<&Path>) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if let Some(arg) = current.take() {
                    args.push(arg);
                }
            }
            '\'' => {
                let arg = current.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => arg.push(c),
                        None => return Err(t!("format-command-unclosed-single-quote").to_owned()),
                    }
                }
            }
            '"' => {
                let arg = current.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                            arg.extend(chars.next());
                        }
                        Some(c) => arg.push(c),
                        None => return Err(t!("format-command-unclosed-double-quote").to_owned()),
                    }
                }
            }
            '~' if current.is_none() && chars.peek() == Some(&'/') => {
                let arg = current.insert(String::new());
                match home {
                    Some(home) => arg.push_str(&home.to_string_lossy()),
                    None => arg.push('~'),
                }
            }
            c => current.get_or_insert_with(String::new).push(c),
        }
    }

    args.extend(current);
    Ok(args)
}

/// 展开 `arg` 中的占位符 [`FILE`] 和 [`RUST_EDITION`]，结果仍是同一个参数。
///
/// 文件路径按 `OsStr` 原样放入（非 UTF-8 的路径也不改变），其中的空格等字符不会把参数拆开。
/// `edition` 为 `None` 时，`{rust-edition}` 展开为 `2021`。
pub(super) fn expand(arg: &str, file: &Path, edition: Option<&str>) -> OsString {
    let mut out = OsString::new();
    let mut rest = arg;

    loop {
        let next = [FILE, RUST_EDITION]
            .into_iter()
            .filter_map(|p| rest.find(p).map(|at| (at, p)))
            .min();
        let Some((at, placeholder)) = next else {
            out.push(rest);
            return out;
        };

        out.push(&rest[..at]);
        if placeholder == FILE {
            out.push(file);
        } else {
            out.push(edition.unwrap_or("2021"));
        }
        rest = &rest[at + placeholder.len()..];
    }
}

#[cfg(test)]
mod tests;
