//! [`resolve_font`] 的测试：字体名与字体文件路径的区分。

use super::*;

#[test]
fn font_request_tells_names_from_paths() {
    assert_eq!(
        resolve_font(Some("Menlo"), "Menlo"),
        FontRequest::Name("Menlo".to_string())
    );
    assert_eq!(
        resolve_font(Some("Helvetica Neue"), "Menlo"),
        FontRequest::Name("Helvetica Neue".to_string())
    );
    assert_eq!(
        resolve_font(Some("/System/Library/Fonts/Menlo.ttc"), "Menlo"),
        FontRequest::Path(PathBuf::from("/System/Library/Fonts/Menlo.ttc"))
    );
    assert_eq!(
        resolve_font(Some("fonts/MyFont.ttf"), "Menlo"),
        FontRequest::Path(PathBuf::from("fonts/MyFont.ttf"))
    );
    assert_eq!(
        resolve_font(None, "Menlo"),
        FontRequest::Name("Menlo".to_string())
    );
    assert_eq!(
        resolve_font(Some("   "), "Menlo"),
        FontRequest::Name("Menlo".to_string())
    );
}
