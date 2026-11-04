//! 编辑器与终端之间命令的测试：路径的引号处理。

use super::*;

#[test]
fn paths_are_quoted_only_when_needed() {
    assert_eq!(shell_quote(Path::new("/a/b-c_d.rs")), "/a/b-c_d.rs");
    assert_eq!(shell_quote(Path::new("/a b/c")), "'/a b/c'");
    assert_eq!(shell_quote(Path::new("it's")), r"'it'\''s'");
    assert_eq!(shell_quote(Path::new("$HOME;rm")), "'$HOME;rm'");
}
