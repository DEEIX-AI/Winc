//! 提交视图底部提交图的测试：位置、各行、点击打开提交和滚轮滚动。

use super::*;

#[test]
fn the_commit_graph_lists_recent_commits_below_the_changes() {
    let Some((mut app, root)) = git_app("git-commit-graph", false) else {
        return;
    };
    history_repo(&root, 40);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let parts = app.commit_parts();
    let graph = parts.graph.expect("有提交时在改动列表下面显示提交图");
    // 上沿接着改动列表，下沿在视图底部或「上一次提交」那一栏上面。
    assert_eq!(parts.list.bottom(), graph.y);
    assert!(graph.bottom() <= app.layout.sidebar.bottom());
    let commits = app.git.graph.commits.clone();
    assert!(!commits.is_empty());
    // 一屏画得下的行都有了点击范围，从上往下就是最近的几个提交。
    let rows: Vec<usize> = app
        .hits
        .commit
        .iter()
        .filter_map(|(_, target)| match target {
            CommitTarget::GraphRow(i) => Some(*i),
            _ => None,
        })
        .collect();
    // 一处一百八十四高，标题占二十四，正好八行。
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[0], 0);
    assert!(rows.windows(2).all(|pair| pair[1] == pair[0] + 1));

    // 点一行在 Git 日志标签里打开这个提交。
    let oid = commits[0].oid.clone();
    app.press_commit(CommitTarget::GraphRow(0));
    let view = app.editor.active().and_then(|d| d.log.as_deref());
    let view = view.expect("打开 Git 日志标签");
    assert!(view.commits.iter().any(|commit| commit.oid == oid));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_graph_scrolls_with_the_wheel() {
    let Some((mut app, root)) = git_app("git-commit-graph-scroll", true) else {
        return;
    };
    history_repo(&root, 40);
    app.run_action(Action::ShowCommit);
    app.git_refresh();
    paint(&mut app);
    let graph = app.commit_parts().graph.expect("显示提交图");
    assert!(app.graph_scroll_max(graph) > 0.0, "提交多于一屏时能滚动");
    assert_eq!(app.git.graph.scroll, 0.0);

    app.pointer.mouse = (graph.x + 30.0, graph.y + 40.0);
    assert_eq!(
        app.scroll_target_at(graph.x + 30.0, graph.y + 40.0),
        ScrollTarget::Graph
    );
    app.on_wheel(MouseScrollDelta::LineDelta(0.0, -2.0));
    paint(&mut app);
    assert!(app.git.graph.scroll > 0.0);

    // 滚过头停在最后：最后一个提交正好挨着下沿。
    for _ in 0..40 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, -5.0));
    }
    paint(&mut app);
    assert_eq!(app.git.graph.scroll, app.graph_scroll_max(graph));

    // 往回滚过头停在最前面。
    for _ in 0..40 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, 5.0));
    }
    paint(&mut app);
    assert_eq!(app.git.graph.scroll, 0.0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 画出具名仓库的提交视图，供人工查看提交图（改动列表多、储藏、上一次提交都在）。
#[test]
#[ignore = "截图，手动运行"]
fn shoot_commit_graph() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let dir = search_folder(&format!("commit-graph-{theme}"));
        if !long_repo(&dir) {
            return;
        }
        history_repo(&dir, 40);
        std::fs::write(dir.join("notes.md"), "# notes\n").unwrap();
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        shoot(&mut app, &format!("commit-graph-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
