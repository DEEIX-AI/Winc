//! `files` 的测试：含中文、空格和目录标记的路径经 NSString、NSURL 往返后内容与类型不变。

use super::*;

#[test]
fn strings_and_urls_cross_over_intact() {
    let text = "存储为：/tmp/中文 名字.txt";
    let string = ns_string(text).unwrap();
    // SAFETY: `UTF8String` 指向的内存在 `string` 存活期间有效，此处立即拷贝。
    let back = unsafe {
        let utf8: *const c_char = msg_send![&*string, UTF8String];
        CStr::from_ptr(utf8).to_str().unwrap().to_owned()
    };
    assert_eq!(back, text);

    for (path, is_dir) in [("/tmp/一个 目录", true), ("/tmp/一个 目录/文件.rs", false)] {
        let url = file_url(Path::new(path), is_dir).unwrap();
        // SAFETY: `file_url` 返回的是 NSURL。
        let back = unsafe { url_path(&url) };
        assert_eq!(back.as_deref(), Some(Path::new(path)));
    }
}
