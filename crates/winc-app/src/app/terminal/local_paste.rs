//! Windows 上粘贴剪贴板图片：用括号粘贴把本机 PNG 路径交给终端程序。

use std::io;
use std::path::Path;
use std::sync::Once;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use winc_backend::fs;

use super::upload::write_new;
use super::*;
use crate::app::NoticeKind;
use crate::app::editor::clipboard::ClipImage;

impl App {
    /// 只有图片、程序开启了括号粘贴并能访问本机路径时，生成一个本机附件。
    pub(super) fn paste_local_terminal_image(&mut self, id: u16, modes: Modes, cwd: &[u8]) -> bool {
        if !modes.has(Modes::BRACKETED_PASTE)
            || self.terminal_at_prompt()
            || self.terminal.tab().is_none_or(|tab| tab.ended.is_some())
            || cwd.starts_with(b"/")
        {
            return false;
        }
        let cwd = bytes_path(cwd);
        if winc_backend::is_remote(&cwd) || winc_editor::wsl::locate(&cwd).is_some() {
            return false;
        }
        let Some(image) = self.clipboard.image() else {
            return false;
        };
        let dir = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(std::env::temp_dir)
            .join("winc")
            .join("terminal-paste");
        match cache_image(&dir, image) {
            Ok(path) => self.confirmed_terminal_paste(id, path),
            Err(error) => self.notify(
                NoticeKind::Error,
                t!("terminal-image-paste-failed", error = error.to_string()),
            ),
        }
        true
    }
}

fn cache_image(dir: &Path, image: ClipImage) -> io::Result<String> {
    let png = image
        .into_png()
        .ok_or_else(|| io::Error::other(t!("terminal-upload-no-image")))?;
    fs::create_dir_all(dir)?;
    static CLEANUP: Once = Once::new();
    let now = SystemTime::now();
    CLEANUP.call_once(|| {
        // 程序可能等到提交会话时才读文件，粘贴后保留一段时间。
        let cutoff = fs::Stamp::from(now - Duration::from_secs(7 * 24 * 60 * 60));
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten().filter(|entry| entry.kind.is_file()) {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "png")
                    && fs::metadata(&path)
                        .is_ok_and(|meta| meta.modified.is_some_and(|stamp| stamp < cutoff))
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
    });
    let stamp = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = write_new(dir, stamp, "paste.png", &png)?;
    // Windows 路径可能包含空格，Codex 和 Claude Code 都能识别双引号内的绝对路径。
    Ok(format!("\"{}\"", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_image_paste_sends_a_quoted_png_in_a_bracketed_paste() {
        let dir = std::env::temp_dir().join(format!("winc paste test {}", std::process::id()));
        let png = winc_editor::paste::encode_png(1, 1, &[12, 34, 56, 255]).unwrap();
        let path = cache_image(&dir, ClipImage::Png(png.clone())).unwrap();
        let second = cache_image(&dir, ClipImage::Png(png.clone())).unwrap();
        let mut modes = Modes::default();
        modes.set(Modes::BRACKETED_PASTE, true);
        let mut bytes = Vec::new();
        input::paste(&path, modes, &mut bytes);
        assert!(bytes.starts_with(b"\x1b[200~\"") && bytes.ends_with(b"\"\x1b[201~"));
        assert_ne!(path, second);
        for text in [path, second] {
            let file = Path::new(text.trim_matches('"'));
            assert_eq!(file.parent(), Some(dir.as_path()));
            assert_eq!(fs::read(file).unwrap(), png);
            fs::remove_file(file).unwrap();
        }
    }
}
