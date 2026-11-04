//! 序列解析辅助函数的测试：base64 解码、`file://` 地址取路径、256 色表的颜色值。

use super::*;

// 编码串带 `=` 填充，解码结果应与原字节一致；出现表外字符 `*` 时返回 `None`。
#[test]
fn base64_decodes_and_rejects_junk() {
    assert_eq!(base64(b"aGVsbG8=").unwrap(), b"hello");
    assert_eq!(base64(b"5Lit5paH").unwrap(), "中文".as_bytes());
    assert!(base64(b"a*b").is_none());
}

// 主机名丢弃，`%20` 解开为空格，UTF-8 的百分号编码还原成汉字；
// 空主机与没有 `://` 的非 URL 输入也分别覆盖。
#[test]
fn file_urls_lose_their_host_and_escapes() {
    assert_eq!(
        file_url_path(b"file://mac.local/Users/a%20b/%E4%B8%AD").unwrap(),
        "/Users/a b/中".as_bytes()
    );
    assert_eq!(file_url_path(b"file:///tmp").unwrap(), b"/tmp");
    assert!(file_url_path(b"/tmp").is_none());
}

// 期望值取自 xterm 的 256 色定义：16 到 231 为 6×6×6 色立方，232 到 255 为灰度阶。
#[test]
fn the_256_colour_cube_matches_xterm() {
    let theme = crate::protocol::Theme::default();
    assert_eq!(theme.color(16), [0, 0, 0]);
    assert_eq!(theme.color(196), [255, 0, 0]);
    assert_eq!(theme.color(231), [255, 255, 255]);
    assert_eq!(theme.color(232), [8, 8, 8]);
    assert_eq!(theme.color(255), [238, 238, 238]);
}
