//! 状态栏的测试：路径（面包屑）及其目录菜单的分段悬停与点击、目录内容的排序与勾选、菜单图标，
//! 以及新版本的提示。

use winc_editor::EntryKind;

use super::*;
use crate::app::status::breadcrumb::{FolderEntry, folder_entries};
use crate::app::status::motion::STATUS_SLIDE;
use std::time::Duration;

#[test]
fn every_crumb_opens_the_folder_it_names() {
    let dir = search_folder("breadcrumb");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    let main = root.join("src/main.rs");
    app.open_path(&main);
    paint(&mut app);

    // 路径由目录名、`src`、`main.rs` 三段组成，每段都可点击。
    let crumbs = app.crumbs();
    let names: Vec<&str> = crumbs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names[1..], ["src", "main.rs"]);
    assert_eq!(crumbs[0].path.as_deref(), Some(root.as_path()));
    assert_eq!(crumbs[2].path.as_deref(), Some(main.as_path()));
    assert_eq!(app.crumb_hits.len(), 3);

    // 鼠标位于某一段内时悬停该段；位于两段之间的箭头上时没有悬停项。
    let (src, _) = app.crumb_hits[1];
    let y = src.y + src.h / 2.0;
    app.on_mouse_move(src.x + 4.0, y);
    assert_eq!(app.hover, Hover::Crumb(1));
    app.on_mouse_move(src.x - 4.0, y);
    assert_eq!(app.hover, Hover::None);

    // 测试中没有窗口（等同没有系统菜单）：点击后菜单画在窗口中，该段保持按下的样子，Esc 关掉。
    app.on_mouse_move(src.x + 4.0, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
    assert!(app.popup().is_some());
    assert_eq!(app.menu_button, Some(crate::app::MenuButton::Crumb(1)));
    app.popup_logical_key(&winit::keyboard::Key::Named(
        winit::keyboard::NamedKey::Escape,
    ));
    assert_eq!(app.menu_button, None);
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(main.as_path())
    );

    // 目录名这一段列出根目录的内容：与文件树一样不列出 `.git`；目录在前，与文件之间用分隔线隔开；
    // 各组内按名字自然排序且不区分大小写；通往当前文件的 `src` 打勾。
    let summary = |entries: Vec<FolderEntry>| -> Vec<String> {
        entries
            .into_iter()
            .map(|e| match e {
                FolderEntry::Item {
                    name,
                    kind,
                    checked,
                    ..
                } => {
                    let slash = if kind == EntryKind::Dir { "/" } else { "" };
                    let tick = if checked { " ✓" } else { "" };
                    format!("{name}{slash}{tick}")
                }
                FolderEntry::Separator => "—".to_owned(),
                FolderEntry::Note(text) => format!("（{text}）"),
            })
            .collect()
    };
    assert_eq!(
        summary(folder_entries(&root, &main)),
        [
            "empty/",
            "src/ ✓",
            "—",
            "a.txt",
            "Cargo.toml",
            "data.bin",
            "Dockerfile",
            "img_CubLwUlc3Adg-NoCRm7Qz2Kp.txt",
            "img_WNoiTOenBOrb4lcV1xYt8.json",
            "short.rs",
        ]
    );
    // `src` 段与 `main.rs` 段列出的都是 `src` 的内容，当前文件打勾。
    assert_eq!(
        summary(folder_entries(&root.join("src"), &main)),
        ["config.rs", "main.rs ✓"]
    );
    assert_eq!(
        summary(folder_entries(&root.join("empty"), &main)),
        ["（空目录）"]
    );
    let gone = folder_entries(&root.join("gone"), &main);
    assert!(matches!(&gone[..], [FolderEntry::Note(text)] if text.starts_with("无法读取：")));

    // 未命名的文档没有路径，只显示名字，不可点击。
    app.new_file();
    paint(&mut app);
    assert_eq!(app.crumbs().len(), 1);
    assert!(app.crumbs()[0].path.is_none());
    assert!(app.crumb_hits.is_empty());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn config_files_show_and_copy_the_key_under_the_caret() {
    let dir = std::env::temp_dir().join(format!("winc-keypath-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("package.json");
    std::fs::write(
        &file,
        "{\n  \"scripts\": {\n    \"build\": \"tsc\"\n  }\n}\n",
    )
    .unwrap();
    let mut app = app_with(&file, false);

    let at = app.editor.active().unwrap().text().line_start(2) + 15;
    app.select(at, at);
    paint(&mut app);
    assert_eq!(app.key_path().as_deref(), Some("scripts.build"));
    app.run_action(crate::keymap::Action::CopyKeyPath);
    assert_eq!(app.copied.as_deref(), Some("scripts.build"));

    // 光标移出键所在的位置后，没有可显示的键路径。
    app.select(0, 0);
    assert_eq!(app.key_path(), None);
    app.run_action(crate::keymap::Action::CopyKeyPath);
    assert_eq!(
        app.status_message.as_deref(),
        Some("光标不在 JSON、YAML 或 TOML 的键中")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_folder_menu_uses_the_same_icons_as_the_file_tree() {
    for dark in [false, true] {
        let dir = sample_folder(if dark {
            "crumb-icons-dark"
        } else {
            "crumb-icons-light"
        });
        let mut app = app_with(&dir, dark);
        paint(&mut app);
        let names = [
            ("src", true),
            ("main.rs", false),
            ("README.md", false),
            ("a.png", false),
        ];
        let icons = crate::app::status::breadcrumb::tests::menu_icons_for(&mut app, &names);
        // 每一项都有图标；文件图标取文件树中该类型的颜色（比较最不透明的像素）。
        for ((name, is_dir), icon) in names.iter().zip(&icons) {
            let icon = icon.as_ref().unwrap_or_else(|| panic!("{name}"));
            let px = *icon.pixels.iter().max_by_key(|&&px| px >> 24).unwrap();
            assert_eq!(px >> 24, 0xff, "{name}");
            let want = if *is_dir {
                app.palette.folder
            } else {
                app.palette.file_icon(name).1
            };
            let got = |shift: u32| f32::from(((px >> shift) & 0xff) as u8) / 255.0;
            assert!((got(16) - want.r).abs() < 0.02, "{name}");
            assert!((got(8) - want.g).abs() < 0.02, "{name}");
            assert!((got(0) - want.b).abs() < 0.02, "{name}");
        }

        // 设置 `WINC_SNAPSHOT_DIR` 时，把图标并排铺在菜单底色上写成 PNG，供人工检查。
        if let Some(dir) = std::env::var_os("WINC_SNAPSHOT_DIR") {
            let (w, h) = (
                icons[0].as_ref().unwrap().width,
                icons[0].as_ref().unwrap().height,
            );
            let bg: u32 = if dark { 0x2b2b30 } else { 0xf2f2f4 };
            let mut sheet = vec![0xff00_0000 | bg; (w * 5 * h) as usize];
            for (k, icon) in icons.iter().flatten().enumerate() {
                for y in 0..h {
                    for x in 0..w {
                        let px = icon.pixels[(y * w + x) as usize];
                        let a = px >> 24;
                        let mix = |s: u32| {
                            let b = (bg >> s) & 0xff;
                            ((px >> s) & 0xff) + b * (255 - a) / 255
                        };
                        sheet[(y * w * 5 + k as u32 * w + x) as usize] =
                            0xff00_0000 | mix(16) << 16 | mix(8) << 8 | mix(0);
                    }
                }
            }
            let image = Image {
                pixels: sheet,
                width: w * 5,
                height: h,
            };
            let name = if dark {
                "crumb-icons-dark"
            } else {
                "crumb-icons-light"
            };
            write_png(&PathBuf::from(dir).join(format!("{name}.png")), &image).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_found_update_shows_in_the_status_bar_and_the_setting_turns_checks_off() {
    let dir = sample_folder("status-update");
    let mut app = app_with(&dir.join("short.rs"), false);
    paint(&mut app);
    let has_update = |app: &App| {
        app.status_hits
            .iter()
            .any(|(_, i)| *i == StatusItem::Update)
    };
    assert!(!has_update(&app));

    // 后台检查找到了新版本：状态栏上有可以点的提示；收起后不再显示。
    *app.shared.update.borrow_mut() = Some("0.1.400".into());
    paint(&mut app);
    assert!(has_update(&app));
    app.open_status_menu(StatusItem::Update);
    assert!(app.context_menu.is_none());
    *app.shared.update.borrow_mut() = None;
    paint(&mut app);
    assert!(!has_update(&app));

    // 定时检查默认开着，设置里可以关掉。
    assert!(app.switch_on(Setting::CheckUpdates));
    app.settings_toggle(Setting::CheckUpdates);
    assert_eq!(app.config.update_auto_check, Some(false));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 没有系统菜单时（macOS 以外的平台，以及没有窗口的测试中）目录菜单画在窗口中：和系统菜单一样
/// 列出该段的目录，打勾的当前文件（或通往它的目录）尽量对准被点击的段，文字和段的文字对齐；子目录
/// 停留时才读出内容展开；选中文件打开它。
#[test]
fn the_folder_menu_is_drawn_in_the_window_without_a_system_menu() {
    use crate::app::PopupRow;
    use winit::keyboard::{Key, NamedKey};

    let dir = search_folder("breadcrumb-popup");
    std::fs::create_dir_all(dir.join("src/nested")).unwrap();
    std::fs::write(dir.join("src/nested/deep.rs"), "fn deep() {}\n").unwrap();
    let mut app = app_with(&dir, false);
    let root = app.editor.workspace.as_ref().unwrap().root.clone();
    let main = root.join("src/main.rs");
    app.open_path(&main);
    paint(&mut app);

    // 点 `main.rs` 一段：列出 `src` 的内容，目录在前、文件在后，`main.rs` 打勾。
    let (crumb, _) = app.crumb_hits[2];
    click(&mut app, (crumb.x + 4.0, crumb.y + crumb.h / 2.0));
    paint(&mut app);
    let level = app.popup().expect("窗口内的目录菜单").levels[0].clone();
    let names: Vec<String> = (level.rows.iter())
        .map(|row| match row {
            PopupRow::Entry {
                name, dir, checked, ..
            } => format!(
                "{name}{}{}",
                if *dir { "/" } else { "" },
                if *checked { " ✓" } else { "" }
            ),
            PopupRow::Separator => "—".into(),
            _ => "?".into(),
        })
        .collect();
    assert_eq!(names, ["nested/", "—", "config.rs", "main.rs ✓"]);
    // 打勾的一项是最后一项：对准状态栏会伸出窗口，整个菜单往上挪，留在窗口中。
    assert_eq!(level.rect.x + crate::app::POPUP_TEXT_X, crumb.x);
    assert!(level.rect.bottom() <= app.layout.window.bottom());
    assert!(level.row_rects[3].bottom() > crumb.y - 24.0);

    // 停在子目录上：展开它的内容；点中间的文件打开它，菜单关掉。
    let nested = level.row_rects[0];
    app.on_mouse_move(nested.x + 20.0, nested.y + nested.h / 2.0);
    paint(&mut app);
    let sub = app.popup().unwrap().levels[1].clone();
    assert!(
        matches!(&sub.rows[..], [PopupRow::Entry { name, dir: false, .. }] if name == "deep.rs")
    );
    let deep = sub.row_rects[0];
    click(&mut app, (deep.x + 20.0, deep.y + deep.h / 2.0));
    assert!(app.popup().is_none());
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(root.join("src/nested/deep.rs").as_path())
    );

    // 键盘：↓ 移到下一项，→ 展开目录，↩ 打开文件。
    let (crumb, _) = app.crumb_hits[0];
    click(&mut app, (crumb.x + 4.0, crumb.y + crumb.h / 2.0));
    paint(&mut app);
    let press = |app: &mut App, key: NamedKey| {
        app.popup_logical_key(&Key::Named(key));
        paint(app);
    };
    press(&mut app, NamedKey::ArrowDown);
    let first = app.popup().unwrap().levels[0].hover;
    assert!(matches!(
        &app.popup().unwrap().levels[0].rows[first.unwrap()],
        PopupRow::Entry { dir: true, .. }
    ));
    press(&mut app, NamedKey::Escape);
    assert!(app.popup().is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 截图：状态栏右侧各项的过渡。
///
/// 依次截取：未打开文件时；打开文件后各项出现途中（60 ms、120 ms）；选中文字后光标位置一项
/// 变宽、新文字淡入的途中（160 ms）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_status_motion() {
    let dir = sample_folder("status-motion");
    let path = dir.join("notes.txt");
    std::fs::write(&path, "hello world\n".repeat(40)).unwrap();
    let mut app = app_with(&dir, false);
    app.status_motion.animate_without_window = true;
    shoot(&mut app, "status-motion-empty");

    app.open_path(&path);
    paint(&mut app);
    for ms in [60, 120] {
        app.status_motion.freeze(Duration::from_millis(ms));
        shoot(&mut app, &format!("status-motion-enter-{ms}"));
    }
    app.status_motion.freeze(STATUS_SLIDE);
    app.status_motion.settle();

    app.select(0, 300);
    paint(&mut app);
    app.status_motion.freeze(Duration::from_millis(160));
    shoot(&mut app, "status-motion-select-160");
    let _ = std::fs::remove_dir_all(&dir);
}
