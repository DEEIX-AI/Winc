//! `system` 的测试：交给系统打开的链接只放行网页和邮件。

use super::*;

#[test]
fn only_web_and_mail_links_go_to_the_system() {
    for url in ["https://docs.rs", "HTTP://x", "mailto:a@b.c"] {
        assert!(openable_url(url), "{url}");
        assert!(open_url(url).is_ok(), "{url}");
    }
    for url in [
        "file:///Applications/Calculator.app",
        "smb://host/share",
        "-a/Applications/Calculator.app",
        "/tmp/x.command",
        "x-man-page://ls",
        "",
    ] {
        assert!(!openable_url(url), "{url}");
        let error = open_url(url).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{url}");
    }
}
