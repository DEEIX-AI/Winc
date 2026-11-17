//! 提交视图底部提交图的测试：位置、各行、点击打开提交、滚轮滚动和拖动。

use super::*;

use crate::app::Drag;
use crate::app::git::graph::{GRAPH_ROW, graph_rows};

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
    app.press_graph_row(0);
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
    many_commits(&root, 24);
    app.run_action(Action::ShowCommit);
    app.git_refresh();
    paint(&mut app);
    let graph = app.commit_parts().graph.expect("显示提交图");
    let rows = graph_rows(graph);
    assert!(app.graph_scroll_max(rows) > 100.0, "提交多于一屏时能滚动");
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
    assert_eq!(app.git.graph.scroll, app.graph_scroll_max(rows));

    // 往回滚过头停在最前面。
    for _ in 0..40 {
        app.on_wheel(MouseScrollDelta::LineDelta(0.0, 5.0));
    }
    paint(&mut app);
    assert_eq!(app.git.graph.scroll, 0.0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 在仓库里再提交 `count` 次：提交图上的行多于一屏，可以滚动很长一段。
fn many_commits(root: &Path, count: usize) {
    for i in 0..count {
        std::fs::write(root.join(format!("src/more{i}.rs")), format!("// {i}\n")).unwrap();
        git_in(root, &["add", "-A"]);
        let message = format!("第 {i} 次改动：细小的调整");
        git_in(root, &["commit", "-q", "-m", &message]);
    }
}

#[test]
fn the_commit_graph_follows_the_pointer_while_dragged() {
    let Some((mut app, root)) = git_app("git-commit-graph-drag", true) else {
        return;
    };
    history_repo(&root, 40);
    many_commits(&root, 24);
    app.run_action(Action::ShowCommit);
    app.git_refresh();
    paint(&mut app);
    let graph = app.commit_parts().graph.expect("显示提交图");
    let rows = graph_rows(graph);
    assert!(app.graph_scroll_max(rows) > 100.0, "提交多于一屏时能滚动");
    // 按住第三行往上拖五十像素：内容跟着指针走，松开时算拖动，不打开提交。
    let (x, y) = (graph.x + 40.0, rows.y + 2.5 * GRAPH_ROW);
    app.pointer.mouse = (x, y);
    app.on_mouse_down(MouseButton::Left);
    assert!(
        matches!(app.pointer.drag, Drag::GraphPan { row: Some(2), .. }),
        "按下图上的一行开始拖动"
    );
    app.on_mouse_move(x, y - 50.0);
    paint(&mut app);
    assert_eq!(app.git.graph.scroll, 50.0);
    assert_eq!(app.pointer.hover, Hover::None, "拖动时行不高亮");
    app.on_mouse_up(MouseButton::Left);
    assert!(
        app.editor.active().and_then(|d| d.log.as_deref()).is_none(),
        "拖动过就不打开提交"
    );

    // 往下拖回去，拖过头停在最前面。
    app.pointer.mouse = (x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y + 120.0);
    paint(&mut app);
    assert_eq!(app.git.graph.scroll, 0.0);
    app.on_mouse_up(MouseButton::Left);

    // 往上拖过头停在最后一行贴着下沿。
    app.pointer.mouse = (x, y);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_move(x, y - 900.0);
    paint(&mut app);
    assert_eq!(app.git.graph.scroll, app.graph_scroll_max(rows));
    app.on_mouse_up(MouseButton::Left);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_click_without_moving_opens_the_commit() {
    let Some((mut app, root)) = git_app("git-commit-graph-click", false) else {
        return;
    };
    history_repo(&root, 40);
    app.run_action(Action::ShowCommit);
    paint(&mut app);
    let graph = app.commit_parts().graph.expect("显示提交图");
    let rows = graph_rows(graph);
    let commits = app.git.graph.commits.clone();
    let oid = commits[2].oid.clone();
    // 按下再松开、中间没有移动：打开按下的那一行。
    let (x, y) = (graph.x + 40.0, rows.y + 2.5 * GRAPH_ROW);
    app.pointer.mouse = (x, y);
    app.on_mouse_down(MouseButton::Left);
    assert!(
        app.editor.active().and_then(|d| d.log.as_deref()).is_none(),
        "按下时不打开提交"
    );
    app.on_mouse_up(MouseButton::Left);
    let view = app.editor.active().and_then(|d| d.log.as_deref());
    let view = view.expect("松开时打开 Git 日志标签");
    assert!(view.commits.iter().any(|commit| commit.oid == oid));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_commit_graph_scrollbar_can_be_dragged() {
    let Some((mut app, root)) = git_app("git-commit-graph-bar", true) else {
        return;
    };
    history_repo(&root, 40);
    many_commits(&root, 24);
    app.run_action(Action::ShowCommit);
    app.git_refresh();
    paint(&mut app);
    let graph = app.commit_parts().graph.expect("显示提交图");
    let rows = graph_rows(graph);
    let max = app.graph_scroll_max(rows);
    assert!(max > 100.0);
    let thumb = scrollbar_thumb(rows, 0.0, max + rows.h).expect("能滚动时有滑块");
    // 鼠标移到行区域右端：滚动条优先于它下面的行。
    let (x, y) = (rows.right() - 4.0, thumb.y + thumb.h / 2.0);
    app.on_mouse_move(x, y);
    assert_eq!(app.pointer.hover, Hover::Scrollbar(Scrollbar::Graph));

    // 按住滑块往下拖：内容跟着走。
    app.on_mouse_down(MouseButton::Left);
    assert!(matches!(
        app.pointer.drag,
        Drag::Scrollbar {
            bar: Scrollbar::Graph,
            ..
        }
    ));
    app.on_mouse_move(x, rows.bottom() - 10.0);
    paint(&mut app);
    assert!(app.git.graph.scroll > 0.0);
    assert_eq!(
        app.scroll_target_at(x, y),
        ScrollTarget::Graph,
        "拖着滑块时滚轮仍然滚动提交图"
    );
    app.on_mouse_up(MouseButton::Left);
    // 点轨道上滑块以外的位置时滑块跳到鼠标处。
    app.pointer.mouse = (x, rows.y + 2.0);
    app.on_mouse_down(MouseButton::Left);
    app.on_mouse_up(MouseButton::Left);
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
        many_commits(&dir, 24);
        std::fs::write(dir.join("notes.md"), "# notes\n").unwrap();
        let mut app = app_with(&dir, dark);
        app.settings_toggle(Setting::GitModule);
        app.run_action(Action::ShowCommit);
        paint(&mut app);
        // 滚动一段并停一会儿：滚动条淡入之后才画得出来，截图里能看到。
        app.git.graph.scroll = 40.0;
        for _ in 0..12 {
            paint(&mut app);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        shoot(&mut app, &format!("commit-graph-{theme}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
