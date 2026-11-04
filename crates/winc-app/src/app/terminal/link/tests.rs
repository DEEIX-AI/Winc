//! 终端链接的测试：`file://` 地址换成路径。

use super::*;

#[test]
fn file_urls_become_paths() {
    assert_eq!(
        file_url("file://host/Users/a%20b/x.rs"),
        Some(PathBuf::from("/Users/a b/x.rs"))
    );
    assert_eq!(file_url("https://x"), None);
}
