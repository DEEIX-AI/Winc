//! `trash` 的测试：移入废纸篓时写入记录并生成不重复的名字，路径按 URL 编码。

use super::*;

#[test]
fn items_move_into_the_trash_with_a_record_and_unique_names() {
    let dir = std::env::temp_dir().join(format!("winc-trash-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let trash = dir.join("Trash");
    let work = dir.join("work dir");
    fs::create_dir_all(work.join("folder")).unwrap();
    fs::write(work.join("a.txt"), "one").unwrap();

    let moved = trash_into(&trash, &work.join("a.txt"), false).unwrap();
    assert_eq!(moved, trash.join("files/a.txt"));
    assert!(!work.join("a.txt").exists());
    assert_eq!(fs::read_to_string(&moved).unwrap(), "one");
    let record = fs::read_to_string(trash.join("info/a.txt.trashinfo")).unwrap();
    assert!(record.starts_with("[Trash Info]\nPath="));
    assert!(record.contains("/work%20dir/a.txt\n"), "{record}");
    assert!(record.contains("\nDeletionDate=2"), "{record}");

    // 重名的换成「名字 2」，扩展名保留；目录也能放进去。
    fs::write(work.join("a.txt"), "two").unwrap();
    let second = trash_into(&trash, &work.join("a.txt"), false).unwrap();
    assert_eq!(second, trash.join("files/a 2.txt"));
    assert!(trash.join("info/a 2.txt.trashinfo").exists());
    let folder = trash_into(&trash, &work.join("folder"), false).unwrap();
    assert!(folder.is_dir());

    // 不存在的项：报错，不留下记录。
    assert!(trash_into(&trash, &work.join("missing"), false).is_err());
    assert!(!trash.join("info/missing.trashinfo").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn paths_are_encoded_like_urls() {
    assert_eq!(encode_path(Path::new("/a b/中.rs")), "/a%20b/%E4%B8%AD.rs");
    assert_eq!(encode_path(Path::new("/x-y_z.~")), "/x-y_z.~");
}
