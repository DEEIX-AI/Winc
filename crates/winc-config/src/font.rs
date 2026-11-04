//! 字体设置：字体名与字体文件路径的区分和解析。

use std::path::PathBuf;

/// 解析用户填写的字体设置，返回按字体名还是按文件路径加载。
///
/// 含路径分隔符（`/` 或 `\`），或扩展名为 `.ttf`、`.ttc`、`.otf`（不区分大小写）时按文件路径，
/// 其余按字体名交给系统查找（macOS 用 CoreText，Windows 用 DirectWrite，Linux 用 fontconfig）。
/// `font` 为 `None` 或只有空白时使用 `fallback`。
pub fn resolve_font(font: Option<&str>, fallback: &str) -> FontRequest {
    let Some(text) = font.map(str::trim).filter(|t| !t.is_empty()) else {
        return FontRequest::Name(fallback.to_string());
    };

    let lower = text.to_ascii_lowercase();
    let path_like = text.contains('/')
        || text.contains('\\')
        || lower.ends_with(".ttf")
        || lower.ends_with(".ttc")
        || lower.ends_with(".otf");

    if path_like {
        FontRequest::Path(PathBuf::from(text))
    } else {
        FontRequest::Name(text.to_string())
    }
}

/// 字体设置解析后的去向：交给系统按名字查找，或者直接加载某个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontRequest {
    /// 字体名，交给系统按名字查找。
    Name(String),
    /// 字体文件路径，直接加载该文件。
    Path(PathBuf),
}

#[cfg(test)]
mod tests;
