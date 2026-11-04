//! 文件树 [`Workspace`] 的测试：按变化只重读受影响的目录、改名与移动保持展开、复制不覆盖、
//! 选中跟随行变化、自然排序与按需展开的缓存。

use super::*;

#[test]
fn outside_changes_reread_only_the_folders_they_touch() {
    let root = crate::test_util::scratch("ws-outside");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::create_dir_all(root.join("closed")).unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();
    fs::write(root.join("src/a.rs"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("src/a.rs"));
    let names =
        |ws: &Workspace| -> Vec<String> { ws.rows().iter().map(|e| e.name.clone()).collect() };
    assert_eq!(names(&ws), ["closed", "src", "deep", "a.rs"]);

    // 只修改文件内容、在从未展开过的目录里新增文件：文件树不变。
    fs::write(root.join("src/a.rs"), "changed").unwrap();
    fs::write(root.join("closed/new.txt"), "").unwrap();
    assert!(!ws.refresh_paths(&[root.join("src/a.rs"), root.join("closed/new.txt")]));

    // 新建文件：只重新读取 src，选中项不变。
    fs::write(root.join("src/b.rs"), "").unwrap();
    assert!(ws.refresh_paths(&[root.join("src/b.rs")]));
    assert_eq!(names(&ws), ["closed", "src", "deep", "a.rs", "b.rs"]);
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src/a.rs"));

    // 已展开的目录被删除：它从树中消失，重新创建同名目录后不会自动展开。
    ws.reveal(&root.join("src/deep/x.py"));
    fs::remove_dir_all(root.join("src/deep")).unwrap();
    assert!(ws.refresh_paths(&[root.join("src/deep"), root.join("src/deep/x.py")]));
    assert_eq!(names(&ws), ["closed", "src", "a.rs", "b.rs"]);
    fs::create_dir_all(root.join("src/deep")).unwrap();
    assert!(ws.refresh_paths(&[root.join("src/deep")]));
    let deep = ws.rows().iter().find(|e| e.name == "deep").unwrap();
    assert!(!deep.expanded);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn rename_keeps_expansion_and_rejects_bad_names() {
    let root = crate::test_util::scratch("ws-rename");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();
    fs::write(root.join("readme.md"), "").unwrap();
    fs::write(root.join("other.md"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("src/deep/x.py"));

    // 目录改名：已展开的子目录仍然展开，并选中新名字所在的行。
    let lib = ws.rename(&root.join("src"), "lib").unwrap();
    assert_eq!(lib, root.join("lib"));
    assert!(root.join("lib/deep/x.py").is_file());
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["lib", "deep", "x.py", "other.md", "readme.md"]);
    assert_eq!(ws.selected_entry().unwrap().path, lib);

    // 只改大小写。
    let upper = ws.rename(&root.join("readme.md"), "README.md").unwrap();
    assert!(ws.rows().iter().any(|e| e.name == "README.md"));
    assert_eq!(upper, root.join("README.md"));

    // 重名、空名字、含斜杠、根目录本身都被拒绝，原文件保持不变。
    for bad in ["other.md", "", "  ", "a/b", "..", "."] {
        assert!(ws.rename(&upper, bad).is_err(), "{bad:?}");
    }
    assert!(ws.rename(&root, "x").is_err());
    assert!(upper.is_file());

    // 同名：什么都不做。
    assert_eq!(ws.rename(&upper, "README.md").unwrap(), upper);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn move_into_keeps_expansion_and_refuses_what_cannot_be_moved() {
    let root = crate::test_util::scratch("ws-move");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();
    fs::write(root.join("readme.md"), "a").unwrap();
    fs::write(root.join("lib/readme.md"), "b").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("src/deep/x.py"));

    // 目录移入另一个目录：其中已展开的子目录仍然展开，树展开到新位置并选中它。
    let moved = ws.move_into(&root.join("src"), &root.join("lib")).unwrap();
    assert_eq!(moved, root.join("lib/src"));
    assert!(root.join("lib/src/deep/x.py").is_file());
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["lib", "src", "deep", "x.py", "readme.md", "readme.md"]
    );
    assert_eq!(ws.selected_entry().unwrap().path, moved);

    // 移回根目录。已在目标目录中时什么都不做。
    let back = ws.move_into(&moved, &root).unwrap();
    assert_eq!(back, root.join("src"));
    assert_eq!(ws.move_into(&back, &root).unwrap(), back);

    // 不能移入自身及其子目录，根目录本身不能移动，同名项不被覆盖。
    assert!(ws.move_into(&back, &back).is_err());
    assert!(ws.move_into(&back, &back.join("deep")).is_err());
    assert!(ws.move_into(&root, &back).is_err());
    let clash = ws.move_into(&root.join("readme.md"), &root.join("lib"));
    assert_eq!(clash.unwrap_err().to_string(), "「lib」中已有同名项");
    assert_eq!(fs::read_to_string(root.join("lib/readme.md")).unwrap(), "b");
    assert!(root.join("readme.md").is_file());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn copy_into_copies_folders_and_never_overwrites() {
    let base = crate::test_util::scratch("ws-copy");
    let (root, outside) = (base.join("project"), base.join("outside"));
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::create_dir_all(outside.join("assets/icons")).unwrap();
    fs::write(root.join("lib/readme.md"), "old").unwrap();
    fs::write(outside.join("readme.md"), "new").unwrap();
    fs::write(outside.join("assets/icons/a.svg"), "<svg/>").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();

    // 复制文件：原文件还在，树展开到副本并选中它。
    let copied = ws.copy_into(&outside.join("readme.md"), &root).unwrap();
    assert_eq!(copied, root.join("readme.md"));
    assert_eq!(fs::read_to_string(&copied).unwrap(), "new");
    assert!(outside.join("readme.md").is_file());
    assert_eq!(ws.selected_entry().unwrap().path, copied);

    // 同名项不被覆盖，副本依次改名。
    let second = ws
        .copy_into(&outside.join("readme.md"), &root.join("lib"))
        .unwrap();
    assert_eq!(second, root.join("lib/readme 2.md"));
    assert_eq!(
        fs::read_to_string(root.join("lib/readme.md")).unwrap(),
        "old"
    );
    let third = ws
        .copy_into(&outside.join("readme.md"), &root.join("lib"))
        .unwrap();
    assert_eq!(third, root.join("lib/readme 3.md"));

    // 复制目录：连同其中的内容；不能复制进自身内部，也不能复制到工作区外。
    let assets = ws
        .copy_into(&outside.join("assets"), &root.join("lib"))
        .unwrap();
    assert!(assets.join("icons/a.svg").is_file());
    let lib = root.join("lib");
    assert!(ws.copy_into(&lib, &lib.join("assets")).is_err());
    assert!(ws.copy_into(&root.join("readme.md"), &outside).is_err());

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn selection_follows_its_entry_when_rows_change() {
    let root = crate::test_util::scratch("ws-select");
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("assets/a.png"), "").unwrap();
    fs::write(root.join("assets/b.png"), "").unwrap();
    fs::write(root.join("src/main.rs"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("src/main.rs"));
    let row = |ws: &Workspace, name: &str| ws.rows().iter().position(|e| e.name == name);

    // 展开上方的目录：行号变化，选中项仍是 main.rs。
    ws.toggle(row(&ws, "assets").unwrap());
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src/main.rs"));

    // 折叠它所在的目录：改为选中该目录。
    ws.toggle(row(&ws, "src").unwrap());
    assert_eq!(ws.selected_entry().unwrap().path, root.join("src"));

    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn renaming_onto_a_different_file_that_differs_only_in_case_is_refused() {
    let root = crate::test_util::scratch("ws-case");
    fs::write(root.join("makefile"), "lower").unwrap();
    // 在不区分大小写的文件系统上，这里写入的是同一个文件，无法覆盖被测情形。
    if fs::write(root.join("Makefile"), "upper").is_ok()
        && fs::read_to_string(root.join("makefile")).unwrap() == "lower"
    {
        let mut ws = Workspace::open(&root).unwrap();
        let root = ws.root.clone();
        assert!(ws.rename(&root.join("makefile"), "Makefile").is_err());
        assert_eq!(fs::read_to_string(root.join("Makefile")).unwrap(), "upper");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn natural_order() {
    let mut names = vec!["file10", "File2", "file1", "a", "B"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["a", "B", "file1", "File2", "file10"]);

    // 只差大小写或前导零的名字也有确定的先后，相同的名字才相等。
    let mut names = vec!["file01", "a.rs", "file1", "A.rs", "file001"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, vec!["A.rs", "a.rs", "file1", "file01", "file001"]);
    assert_eq!(natural_cmp("x01y2", "x1y02"), Ordering::Greater);
    assert_eq!(natural_cmp("a.rs", "a.rs"), Ordering::Equal);
}

#[test]
fn lazy_expand_cache_and_reveal() {
    let root = crate::test_util::scratch("ws");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join("README.md"), "").unwrap();
    fs::write(root.join("src/main.py"), "").unwrap();
    fs::write(root.join("src/deep/x.py"), "").unwrap();

    let mut ws = Workspace::open(&root).unwrap();
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["src", "README.md"]);

    ws.toggle(0);
    let names: Vec<_> = ws
        .rows()
        .iter()
        .map(|e| (e.name.as_str(), e.depth))
        .collect();
    assert_eq!(
        names,
        vec![("src", 0), ("deep", 1), ("main.py", 1), ("README.md", 0)]
    );

    // 缓存生效：磁盘上新增的文件在 `refresh` 之前看不到。
    fs::write(root.join("src/new.py"), "").unwrap();
    ws.toggle(0);
    ws.toggle(0);
    assert_eq!(ws.rows().len(), 4);
    ws.refresh();
    assert_eq!(ws.rows().len(), 5);

    let target = winc_config::files::canonicalize(root.join("src/deep/x.py")).unwrap();
    ws.reveal(&target);
    assert_eq!(
        ws.selected.map(|i| ws.rows()[i].name.as_str()),
        Some("x.py")
    );
    // 每一行记着上级和子孙的范围，应与按缩进推算的结果一致。
    let rows = ws.rows();
    for (i, e) in rows.iter().enumerate() {
        let parent = rows[..i].iter().rposition(|p| p.depth < e.depth);
        assert_eq!(e.parent, parent.filter(|_| e.depth > 0), "{}", e.name);
        let end = i
            + 1
            + rows[i + 1..]
                .iter()
                .take_while(|c| c.depth > e.depth)
                .count();
        assert_eq!(e.end, end, "{}", e.name);
    }

    // 新建：选中文件时建在它旁边，可以带多级目录，已存在时不覆盖。
    let dir = ws.target_dir();
    assert_eq!(dir, ws.root.join("src/deep"));
    let made = ws.create(&dir, "a/b.py", EntryKind::File).unwrap();
    assert_eq!(made, ws.root.join("src/deep/a/b.py"));
    assert_eq!(ws.selected_entry().map(|e| e.name.as_str()), Some("b.py"));
    assert!(ws.create(&dir, "a/b.py", EntryKind::File).is_err());
    assert!(ws.create(&dir, "../x", EntryKind::Dir).is_err());
    ws.create(&ws.root.clone(), "docs", EntryKind::Dir).unwrap();
    assert!(ws.root.join("docs").is_dir());

    // 全部折叠后选中最外层的目录。
    ws.reveal(&made);
    ws.collapse_all();
    let names: Vec<_> = ws.rows().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["docs", "src", "README.md"]);
    assert_eq!(ws.selected_entry().map(|e| e.name.as_str()), Some("src"));

    fs::remove_dir_all(&root).unwrap();
}

mod measure {
    use super::*;

    /// 测量：展开着上万行的文件树里切换一个目录（重新拍平全部可见行）要多久。
    ///
    /// `cargo test -p winc-editor --release measure_tree_toggle -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_tree_toggle() {
        let root = crate::test_util::scratch("ws-measure");
        for d in 0..50 {
            let dir = root.join(format!("node_modules/package-{d}"));
            fs::create_dir_all(&dir).unwrap();
            for f in 0..200 {
                fs::write(dir.join(format!("module_file_{f}.js")), "").unwrap();
            }
        }
        let mut ws = Workspace::open(&root).unwrap();
        let root = ws.root.clone();
        for d in 0..50 {
            ws.reveal(&root.join(format!("node_modules/package-{d}/module_file_0.js")));
        }
        let rows = ws.rows().len();
        let last = rows - 201;
        assert_eq!(ws.rows()[last].name, "package-49");
        let start = std::time::Instant::now();
        for _ in 0..100 {
            ws.toggle(last);
        }
        eprintln!("{rows} 行：{:?} / 次", start.elapsed() / 100);
        fs::remove_dir_all(&root).unwrap();
    }
}

/// 回归：去掉上级目录也选中了的项时两两比较，全选上万项时是平方级。
///
/// 结果按显示顺序，只留最上层的项。
#[test]
fn selection_roots_drop_entries_under_selected_folders() {
    let root = crate::test_util::scratch("ws-roots");
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::create_dir_all(root.join("a2")).unwrap();
    fs::write(root.join("a/b/x.txt"), "").unwrap();
    fs::write(root.join("a/y.txt"), "").unwrap();
    fs::write(root.join("a2/z.txt"), "").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("a/b/x.txt"));
    ws.reveal(&root.join("a2/z.txt"));
    // `a2` 的名字以 `a` 开头，但不在 `a` 下面；`a/b/x.txt` 和 `a/y.txt` 都在选中的 `a` 下面。
    for path in ["a", "a/b", "a/b/x.txt", "a/y.txt", "a2/z.txt"] {
        ws.marked.insert(root.join(path));
    }
    assert_eq!(
        ws.selection_roots(),
        [root.join("a"), root.join("a2/z.txt")]
    );
    ws.marked.remove(&root.join("a"));
    assert_eq!(
        ws.selection_roots(),
        [
            root.join("a/b"),
            root.join("a/y.txt"),
            root.join("a2/z.txt")
        ]
    );
    let _ = fs::remove_dir_all(&root);
}

/// 回归：只能移动本工作区里的文件，在另一个窗口的文件树里剪切的文件粘贴过来总是失败。
#[test]
fn entries_from_outside_the_workspace_can_be_moved_in() {
    let root = crate::test_util::scratch("ws-move-in");
    let outside = crate::test_util::scratch("ws-move-in-other");
    fs::write(outside.join("notes.txt"), "笔记").unwrap();
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();

    let moved = ws.move_into(&outside.join("notes.txt"), &root).unwrap();
    assert_eq!(moved, root.join("notes.txt"));
    assert_eq!(fs::read_to_string(&moved).unwrap(), "笔记");
    assert_eq!(ws.selected_entry().unwrap().path, moved);
    // 目标仍须在工作区里。
    fs::write(root.join("back.txt"), "").unwrap();
    assert!(ws.move_into(&root.join("back.txt"), &outside).is_err());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

/// 回归：拖动时每帧在全部行里逐个找放置目标所在的行。
///
/// 逐层往下找的结果与逐个找相同；没有展开到的、不在工作区里的找不到。
#[test]
fn rows_are_found_by_walking_down_the_tree() {
    let root = crate::test_util::scratch("ws-row-of");
    for path in [
        "a/b/c/x.txt",
        "a/b/y.txt",
        "a/z.txt",
        "a2/w.txt",
        "d/e/v.txt",
        "top.txt",
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let mut ws = Workspace::open(&root).unwrap();
    let root = ws.root.clone();
    ws.reveal(&root.join("a/b/c/x.txt"));
    ws.reveal(&root.join("a2/w.txt"));
    for (i, entry) in ws.rows().iter().enumerate() {
        assert_eq!(ws.row_of(&entry.path), Some(i), "{}", entry.path.display());
    }
    // `d` 没展开：它下面的行不在可见行里。
    assert_eq!(ws.row_of(&root.join("d/e")), None);
    assert_eq!(ws.row_of(&root.join("nowhere")), None);
    assert_eq!(ws.row_of(Path::new("/elsewhere")), None);
    let _ = fs::remove_dir_all(&root);
}
