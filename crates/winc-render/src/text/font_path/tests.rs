//! 字体名解析的测试：路径与名字的检查、CoreText 查询，以及系统保留名的处理。

use super::*;

#[test]
fn path_spec_requires_an_existing_file() {
    let existing = std::env::current_exe().expect("测试进程自己的路径总是存在");
    assert_eq!(
        FontSpec::Path(existing.clone()).resolve(),
        Some(FontFile::first(existing))
    );

    assert_eq!(
        FontSpec::Path(PathBuf::from("/nope/nowhere.ttf")).resolve(),
        None
    );
}

#[test]
fn check_tells_a_missing_font_from_one_that_cannot_be_drawn() {
    use crate::text::font::tests::bare_font;

    let dir = std::env::temp_dir().join(format!("winc-font-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hvgl = dir.join("hvgl.ttf");
    std::fs::write(&hvgl, bare_font(&[b"cmap", b"hmtx", b"hvgl"])).unwrap();
    let glyf = dir.join("glyf.ttf");
    std::fs::write(&glyf, bare_font(&[b"cmap", b"glyf"])).unwrap();

    assert_eq!(FontSpec::Path(glyf).check(), Ok(()));
    assert_eq!(FontSpec::Path(hvgl).check(), Err(FontError::Unsupported));
    assert_eq!(
        FontSpec::Path(dir.join("missing.ttf")).check(),
        Err(FontError::NotFound)
    );
    assert_eq!(
        FontSpec::Name("winc 不存在的字体 42".into()).check(),
        Err(FontError::NotFound)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_font_name_resolves_to_nothing() {
    // 名字中带随机因素，不会被真实字体使用，任何平台上都不应解析出文件。
    assert_eq!(
        FontSpec::Name("winc 不存在的字体 42".into()).resolve(),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn coretext_finds_the_system_monospace_font() {
    let found = FontSpec::Name("Menlo".into())
        .resolve()
        .expect("Menlo 是 macOS 自带字体");
    assert!(found.path.to_string_lossy().contains("Menlo"));
    // 字族名取常规体；PostScript 名指定的是集合中的哪一个字体。
    assert_eq!(found.postscript.as_deref(), Some("Menlo-Regular"));
    let bold = FontSpec::Name("Menlo-Bold".into()).resolve().unwrap();
    assert_eq!(bold.postscript.as_deref(), Some("Menlo-Bold"));
}

#[cfg(target_os = "macos")]
#[test]
fn coretext_does_not_report_a_substitute_as_a_hit() {
    // 按名字创建字体时，CoreText 对不认识的名字会改用 Helvetica，前缀也会被当成 Helvetica；
    // 这两种结果都不算找到。以 `.` 开头的系统保留名同理。
    for name in ["Winc 绝不会装的字体", "H", "Helvet", ".SF NS Mono", ""] {
        assert_eq!(FontSpec::Name(name.into()).resolve(), None, "{name:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn coretext_accepts_localized_family_names() {
    let found = FontSpec::Name("苹方-简".into())
        .resolve()
        .expect("苹方是 macOS 自带字体");
    assert_eq!(found.postscript.as_deref(), Some("PingFangSC-Regular"));
}

#[cfg(target_os = "macos")]
#[test]
fn font_files_with_spaces_in_their_names_are_found() {
    let found = FontSpec::Name("Hiragino Sans GB".into()).resolve();
    let found = found.expect("冬青黑体是 macOS 自带字体");
    assert!(
        found.path.ends_with("Hiragino Sans GB.ttc"),
        "{:?}",
        found.path
    );
}

#[cfg(target_os = "macos")]
#[test]
fn system_font_names_map_to_the_system_font_files() {
    let mono = FontSpec::Name("SF Mono".into()).resolve().unwrap();
    assert!(mono.path.ends_with("SFNSMono.ttf"));
    let ui = FontSpec::Name("sf pro".into()).resolve().unwrap();
    assert!(ui.path.ends_with("SFNS.ttf"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn linux_and_windows_scan_font_directories() {
    // 只验证调用不会 panic：这两个平台上能否找到字体取决于系统安装了哪些字体。
    let _ = FontSpec::Name("DejaVu Sans Mono".into()).resolve();
}
