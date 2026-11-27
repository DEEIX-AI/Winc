//! Git 更改列表的跨平台显示模式测试，不依赖外部 Git 进程。

use super::*;
use crate::app::input::context_menu::MenuEntry;
use crate::app::{CommitRow, CommitTarget, OtherGroup, OtherRow, PopupRow};
use winc_editor::git::{Change, Entry, Head, Status};

#[test]
fn git_layout_menu_rebuilds_and_draws_the_current_repository_tree() {
    let dir = sample_folder("git-layout-menu");
    let mut app = app_with(&dir, false);
    let root = app.editor.root().unwrap().to_path_buf();
    app.git.root = Some(root.clone());
    app.git.status = Some(Status {
        head: Some(Head::Branch("main".into())),
        entries: ["src/main.rs", "src/nested/lib.rs", "Cargo.toml"]
            .into_iter()
            .map(|path| Entry {
                path: root.join(path),
                from: None,
                staged: None,
                unstaged: Some(Change::Modified),
                is_dir: false,
            })
            .collect(),
        ..Status::default()
    });
    app.panels.sidebar.view = SidebarView::Commit;
    paint(&mut app);
    assert!(
        !app.commit_rows()
            .iter()
            .any(|r| matches!(r, CommitRow::Dir { .. }))
    );

    for command in [MenuCommand::CommitTree, MenuCommand::CommitList] {
        // 模式切换不能继续绘制展开动画里保存的旧版列表。
        app.git.commit.motion = Some(crate::app::git::commit::CommitMotion {
            rows: app.commit_rows(),
            parent: 0,
            count: 3,
            expanding: true,
            clock: crate::app::Clock::stopped(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_millis(500),
            ),
        });
        app.press_commit(CommitTarget::More);
        paint(&mut app);
        let menu = app.overlays.context_menu.as_ref().unwrap();
        let top = &app.popup().unwrap().levels[0];
        let index = top
            .rows
            .iter()
            .position(|row| match row {
                PopupRow::Submenu { rows, .. } => rows.iter().any(|row| {
                    matches!(row, PopupRow::Item { tag, .. }
                    if MenuEntry::command_at(&menu.entries, *tag) == Some(command))
                }),
                _ => false,
            })
            .unwrap();
        let at = top.row_rects[index];
        click(&mut app, (at.x + 20.0, at.y + at.h / 2.0));
        paint(&mut app);
        let menu = app.overlays.context_menu.as_ref().unwrap();
        let sub = &app.popup().unwrap().levels[1];
        let index = sub
            .rows
            .iter()
            .position(|row| {
                matches!(row, PopupRow::Item { tag, .. }
                if MenuEntry::command_at(&menu.entries, *tag) == Some(command))
            })
            .unwrap();
        let at = sub.row_rects[index];
        click(&mut app, (at.x + 20.0, at.y + at.h / 2.0));
        paint(&mut app);
        let tree = command == MenuCommand::CommitTree;
        assert_eq!(app.state.commit_tree, tree);
        assert!(app.menu_checked(command));
        assert!(app.git.commit.motion.is_none());
        assert!(app.git.commit.fold_motion.is_none());
        assert_eq!(
            app.commit_rows()
                .iter()
                .any(|r| matches!(r, CommitRow::Dir { .. })),
            tree
        );
        assert_eq!(
            app.hits
                .commit
                .iter()
                .any(|(_, t)| matches!(t, CommitTarget::Dir(_))),
            tree
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn git_layout_tree_applies_to_other_repositories() {
    let folder = sample_folder("git-layout-others");
    let mut app = app_with(&folder, false);
    let root = app.editor.root().unwrap().to_path_buf();
    let other = root.join("other");
    app.git.root = Some(root.clone());
    app.git.repositories = vec![root.clone(), other.clone()];
    app.git.status = Some(Status::default());
    app.state.commit_all_repositories = Some(true);
    app.git.summary_for_test(
        other.clone(),
        Status {
            entries: ["src/main.rs", "src/nested/lib.rs", "Cargo.toml"]
                .into_iter()
                .map(|path| Entry {
                    path: other.join(path),
                    from: None,
                    staged: None,
                    unstaged: Some(Change::Modified),
                    is_dir: false,
                })
                .collect(),
            ..Status::default()
        },
    );
    let flat = app.commit_other_rows();
    app.toggle_commit_layout();
    let tree = app.commit_other_rows();
    assert!(tree.len() > flat.len(), "树形模式应显示其他仓库的目录行");
    assert!(
        tree.iter()
            .any(|r| matches!(r, OtherRow::TreeFile(1, 1, OtherGroup::Changes, 2)))
    );
    app.panels.sidebar.view = SidebarView::Commit;
    paint(&mut app);
    if std::env::var_os("WINC_SNAPSHOT_DIR").is_some() {
        shoot(&mut app, "git-layout-other-tree");
    }
    let dir = tree
        .iter()
        .position(|r| matches!(r, OtherRow::Dir(_, _, path, _, 0) if path == Path::new("src")))
        .unwrap();
    let hit = app
        .hits
        .commit
        .iter()
        .find(|(_, t)| *t == CommitTarget::OtherDir(dir))
        .unwrap()
        .0;
    click(&mut app, (hit.x + 80.0, hit.y + hit.h / 2.0));
    assert_eq!(app.git.root, Some(root));
    assert!(
        !app.commit_other_rows()
            .iter()
            .any(|r| matches!(r, OtherRow::TreeFile(1, 0 | 1, ..)))
    );
    assert!(
        app.commit_other_rows()
            .iter()
            .any(|r| matches!(r, OtherRow::TreeFile(1, 2, ..)))
    );
    app.run_commit_more_command(MenuCommand::CommitExpandAll);
    assert_eq!(app.commit_other_rows(), tree);
    app.toggle_commit_layout();
    assert_eq!(app.commit_other_rows(), flat);

    // 只暂存了一部分的文件分别出现在两组，目录的折叠互不影响。
    app.git.summary_for_test(
        other.clone(),
        Status {
            entries: vec![Entry {
                path: other.join("src/main.rs"),
                from: None,
                staged: Some(Change::Modified),
                unstaged: Some(Change::Modified),
                is_dir: false,
            }],
            ..Status::default()
        },
    );
    app.git.commit.set_staging(true);
    app.toggle_commit_layout();
    let rows = app.commit_other_rows();
    for group in [OtherGroup::Staged, OtherGroup::Changes] {
        assert!(
            rows.iter()
                .any(|r| matches!(r, OtherRow::TreeFile(1, 0, g, 1) if *g == group))
        );
    }
    let dir = rows
        .iter()
        .position(|r| matches!(r, OtherRow::Dir(_, OtherGroup::Staged, ..)))
        .unwrap();
    app.press_commit(CommitTarget::OtherDir(dir));
    let rows = app.commit_other_rows();
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r, OtherRow::TreeFile(_, _, OtherGroup::Staged, _)))
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r, OtherRow::TreeFile(_, _, OtherGroup::Changes, _)))
    );
    let _ = std::fs::remove_dir_all(&folder);
}

/// 暂存区视图按暂存区和工作区分组；切换视图只改变分组，不改变仓库状态。
#[test]
fn staging_groups_partition_index_and_worktree_changes() {
    use crate::app::CommitGroup::{Changes, Conflicts, Staged, Untracked};

    let folder = sample_folder("git-staging-groups");
    let mut app = app_with(&folder, false);
    let root = app.editor.root().unwrap().to_path_buf();
    let status = Status {
        entries: [
            ("src/added.rs", Some(Change::Added), None),
            ("src/modified.rs", None, Some(Change::Modified)),
            (
                "src/partial.rs",
                Some(Change::Modified),
                Some(Change::Modified),
            ),
            ("src/untracked.rs", None, Some(Change::Untracked)),
            ("src/conflict.rs", None, Some(Change::Conflicted)),
        ]
        .into_iter()
        .map(|(path, staged, unstaged)| Entry {
            path: root.join(path),
            from: None,
            staged,
            unstaged,
            is_dir: false,
        })
        .collect(),
        ..Status::default()
    };
    app.git.root = Some(root);
    app.git.status = Some(status.clone());
    app.settings_toggle(Setting::GitStagingArea);

    let rows = app.commit_rows();
    assert_eq!(rows.group(Staged), [0, 2]);
    assert_eq!(rows.group(Changes), [1, 2, 3]);
    assert_eq!(rows.group(Conflicts), [4]);
    assert!(rows.group(Untracked).is_empty());

    // 显式切回勾选式会重新分组，但不能改动暂存区或工作区的状态。
    app.settings_toggle(Setting::GitStagingArea);
    let rows = app.commit_rows();
    assert!(rows.group(Staged).is_empty());
    assert_eq!(rows.group(Changes), [0, 1, 2]);
    assert_eq!(rows.group(Untracked), [3]);
    assert_eq!(app.git.status.as_ref(), Some(&status));

    std::fs::remove_dir_all(&folder).unwrap();
}

/// 统计某个矩形区域内呈 `target` 色相的像素数量（用于验证 `+N −M` 绘制与隐藏，排除中性灰按钮图标）。
fn tinted_pixels(app: &App, rect: Rect, target: Color) -> usize {
    let image = app.frame.renderer.as_ref().unwrap().image().unwrap();
    let x0 = (rect.x * SCALE).floor().max(0.0) as usize;
    let x1 = (rect.right() * SCALE).ceil().min(image.width as f32) as usize;
    let y0 = (rect.y * SCALE).floor().max(0.0) as usize;
    let y1 = (rect.bottom() * SCALE).ceil().min(image.height as f32) as usize;
    let (tr, tg, tb) = (
        (target.r * 255.0) as i32,
        (target.g * 255.0) as i32,
        (target.b * 255.0) as i32,
    );
    let mut hits = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let px = image.pixels[y * image.width as usize + x];
            let (r, g, b) = (
                ((px >> 16) & 0xff) as i32,
                ((px >> 8) & 0xff) as i32,
                (px & 0xff) as i32,
            );
            let chroma = r.max(g).max(b) - r.min(g).min(b);
            if chroma >= 25 && (r - tr).abs() + (g - tg).abs() + (b - tb).abs() < 120 {
                hits += 1;
            }
        }
    }
    hits
}

/// 提交面板文件行与分组标题行显示 `+N −M`，悬停时由操作按钮替代，窄侧栏时自动隐藏。
#[test]
fn commit_rows_draw_diff_stats_and_swap_on_hover_or_narrow_sidebar() {
    use crate::app::CommitGroup;
    use winc_editor::git::DiffStats;

    let folder = sample_folder("git-commit-diff-stats");
    let mut app = app_with(&folder, true);
    let root = app.editor.root().unwrap().to_path_buf();
    let mod_path = root.join("a/b/c/d/main.rs");
    let untracked_path = root.join("notes.txt");
    let mut diff_stats = DiffStats::default();
    diff_stats.unstaged.insert(mod_path.clone(), (1234, 567));
    app.git.root = Some(root);
    app.git.status = Some(Status {
        head: Some(Head::Branch("dev".into())),
        entries: vec![
            Entry {
                path: mod_path,
                from: None,
                staged: None,
                unstaged: Some(Change::Modified),
                is_dir: false,
            },
            Entry {
                path: untracked_path,
                from: None,
                staged: None,
                unstaged: Some(Change::Untracked),
                is_dir: false,
            },
        ],
        diff_stats,
        ..Status::default()
    });
    app.panels.sidebar.width = 280.0;
    app.panels.sidebar.view = SidebarView::Commit;
    paint(&mut app);

    let hit = |app: &App, target: CommitTarget| {
        app.hits
            .commit
            .iter()
            .find(|(_, t)| *t == target)
            .map(|(r, _)| *r)
    };
    let group_row = hit(&app, CommitTarget::Group(CommitGroup::Changes)).unwrap();
    let file_row = hit(&app, CommitTarget::File(0)).unwrap();
    let untracked_row = hit(&app, CommitTarget::File(1)).unwrap();

    // 状态字母与文件数徽标占最右 ~24px，统计文字位于其左侧一段。
    let stat_band = |row: Rect| Rect::new(row.right() - 110.0, row.y, 84.0, row.h);
    let added_color = app.palette.git_added;
    let deleted_color = app.palette.git_deleted;

    assert!(tinted_pixels(&app, stat_band(file_row), added_color) > 0);
    assert!(tinted_pixels(&app, stat_band(file_row), deleted_color) > 0);
    assert!(tinted_pixels(&app, stat_band(group_row), added_color) > 0);
    assert!(tinted_pixels(&app, stat_band(group_row), deleted_color) > 0);
    assert_eq!(
        tinted_pixels(&app, stat_band(untracked_row), added_color),
        0
    );

    // 悬停文件行：按钮替代 `+N −M`，统计色像素消失。
    app.on_mouse_move(file_row.x + 50.0, file_row.y + file_row.h / 2.0);
    paint(&mut app);
    assert!(hit(&app, CommitTarget::OpenFile(0)).is_some());
    assert_eq!(tinted_pixels(&app, stat_band(file_row), added_color), 0);
    assert_eq!(tinted_pixels(&app, stat_band(file_row), deleted_color), 0);

    // 悬停分组行：整组按钮替代汇总 `+A −D`。
    app.on_mouse_move(group_row.x + 50.0, group_row.y + group_row.h / 2.0);
    paint(&mut app);
    assert!(hit(&app, CommitTarget::GroupRevert(CommitGroup::Changes)).is_some());
    assert_eq!(tinted_pixels(&app, stat_band(group_row), added_color), 0);
    assert_eq!(tinted_pixels(&app, stat_band(group_row), deleted_color), 0);

    // 移开鼠标、切树形并收窄到最小宽度：留给文件名的空间不足时自动隐藏 `+N −M`。
    app.on_mouse_move(app.layout.window.right() - 20.0, 20.0);
    app.git.commit.set_flat(false);
    app.panels.sidebar.width = crate::theme::MIN_PANEL_WIDTH;
    paint(&mut app);
    let narrow_file = hit(&app, CommitTarget::File(0)).unwrap();
    assert_eq!(tinted_pixels(&app, stat_band(narrow_file), added_color), 0);
    assert_eq!(
        tinted_pixels(&app, stat_band(narrow_file), deleted_color),
        0
    );

    let _ = std::fs::remove_dir_all(&folder);
}
