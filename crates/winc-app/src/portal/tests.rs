//! `portal` 的测试：文件 URI 转换为路径。

use super::*;

#[test]
fn file_uris_become_paths() {
    assert_eq!(
        file_path("file:///home/me/a%20b/%E4%B8%AD.rs"),
        Some(PathBuf::from("/home/me/a b/中.rs"))
    );
    assert_eq!(
        file_path("file://localhost/tmp/x"),
        Some(PathBuf::from("/tmp/x"))
    );
    assert_eq!(file_path("https://example.com/x"), None);
    assert_eq!(file_path("file:///bad%2"), None);
}
