//! 状态栏路径的测试：点击路径段列出的目录、项数过多时的截断。

use super::*;

/// 测试用：按弹出目录菜单时的做法画好图标，取出 `names`（名字，是否目录）各项的图标。
pub(in crate::app) fn menu_icons_for(app: &mut App, names: &[(&str, bool)]) -> Vec<Option<Image>> {
    let icons = app.folder_icons();
    FOLDER_ICONS.set(icons);
    let out = names.iter().map(|&(n, d)| folder_icon(n, d)).collect();
    FOLDER_ICONS.set(None);
    out
}

#[test]
fn a_crumb_lists_itself_and_the_file_lists_its_folder() {
    let file = Path::new("/p/src/main.rs");
    assert_eq!(crumb_folder(Path::new("/p"), file), Some(Path::new("/p")));
    assert_eq!(
        crumb_folder(Path::new("/p/src"), file),
        Some(Path::new("/p/src"))
    );
    assert_eq!(crumb_folder(file, file), Some(Path::new("/p/src")));
}

#[test]
fn huge_folders_are_cut_short_with_a_note() {
    let dir = std::env::temp_dir().join(format!("winc-folder-menu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..FOLDER_MENU_MAX + 5 {
        std::fs::write(dir.join(format!("f{i:04}.txt")), "").unwrap();
    }

    let entries = folder_entries(&dir, &dir.join("f0000.txt"));
    let items = entries
        .iter()
        .filter(|e| matches!(e, FolderEntry::Item { .. }))
        .count();
    assert_eq!(items, FOLDER_MENU_MAX);
    assert_eq!(
        entries.last(),
        Some(&FolderEntry::Note("还有 5 项未列出".to_owned()))
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
