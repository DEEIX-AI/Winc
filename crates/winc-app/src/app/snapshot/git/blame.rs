//! 逐行追溯的测试：行号栏标注、未提交的行、悬停提示与跳到日志中的提交。

use super::*;

/// 在 `root` 中以作者 `who`（姓名、邮箱）提交 `file`，提交时间设为 `days` 天前。
fn dated_commit(root: &Path, file: &str, text: &str, message: &str, who: (&str, &str), days: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let date = format!("{} +0800", now - days * 86_400);
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    git_in(root, &["add", "."]);
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", &format!("user.name={}", who.0), "-c"])
        .arg(format!("user.email={}", who.1))
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok, "commit {message}");
}

/// 为 `src/server.rs` 生成三次提交：两个作者分别修改监听地址与连接处理，供逐行追溯使用。
fn blame_repo(root: &Path) {
    let zhang = ("张伟", "zhang@example.com");
    let alice = ("Alice Chen", "alice@example.com");
    let v1 = "use std::net::TcpListener;\n\nfn main() {\n    let listener = TcpListener::bind(\"127.0.0.1:8080\").unwrap();\n    for stream in listener.incoming() {\n        handle(stream.unwrap());\n    }\n}\n";
    dated_commit(
        root,
        "src/server.rs",
        v1,
        "服务器：监听 8080 端口",
        zhang,
        120,
    );
    let v2 = v1.replace("127.0.0.1:8080", "0.0.0.0:8080");
    dated_commit(root, "src/server.rs", &v2, "服务器：对外网开放", alice, 30);
    let v3 = v2.replace(
        "        handle(stream.unwrap());\n",
        "        let stream = stream.unwrap();\n        log::info!(\"连接 {:?}\", stream.peer_addr());\n        handle(stream);\n",
    );
    dated_commit(root, "src/server.rs", &v3, "记录每个连接的来源", zhang, 1);
}

#[test]
fn blame_annotates_lines_and_jumps_to_the_commit_in_the_log() {
    use crate::app::git::blame::BLAME_W;

    let Some((mut app, root)) = git_app("git-blame", false) else {
        return;
    };
    blame_repo(&root);
    app.git_refresh();
    app.open_path(&root.join("src/server.rs"));
    paint(&mut app);
    let width = app.layout.gutter.w;

    // 打开：行号栏多出一栏 `BLAME_W`；每一行的作者对应到各自的提交。
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    assert_eq!(app.layout.gutter.w, width + BLAME_W);
    let id = app.editor.active().unwrap().id;
    let blame = app.blame_of(id).expect("算好了");
    let summary = |line: usize| blame.commits[blame.lines[line] as usize].summary.clone();
    assert_eq!(summary(0), "服务器：监听 8080 端口");
    assert_eq!(summary(3), "服务器：对外网开放");
    assert_eq!(summary(5), "记录每个连接的来源");

    // 修改一行但不保存：重算后该行标记为「未提交」，其余行的归属不变。
    let at = app.editor.active().unwrap().text().line_start(1);
    app.select(at, at);
    app.run(winc_editor::Command::Insert("// 新加的一行\n".into()));
    paint(&mut app);
    let blame = app.blame_of(id).unwrap();
    assert!(blame.commits[blame.lines[1] as usize].uncommitted);
    assert_eq!(
        blame.commits[blame.lines[4] as usize].summary,
        "服务器：对外网开放"
    );

    // 悬停提示写着提交号和说明；点了在日志里选中那次提交。
    let l = app.layout;
    let y = l.text.y + 4.5 * app.fonts.line_height;
    let x = l.gutter.x + 40.0;
    assert_eq!(app.hover_at(x, y), Hover::Blame(4));
    let tip = app.blame_tip(4).unwrap().1;
    assert!(tip.contains("服务器：对外网开放"), "{tip}");
    click(&mut app, (x, y));
    assert!(app.log_active());
    let view = app.editor.active().unwrap().log.as_deref().unwrap();
    let selected = view.selected.map(|i| view.commits[i].subject.clone());
    assert_eq!(selected.as_deref(), Some("服务器：对外网开放"));

    // 回到文件再关闭追溯：行号栏恢复原来的宽度。
    app.open_path(&root.join("src/server.rs"));
    assert!(app.run_action(Action::GitBlame));
    paint(&mut app);
    assert_eq!(app.layout.gutter.w, width);
    let _ = std::fs::remove_dir_all(&root);
}

/// 截图：逐行追溯（`blame-light`、`blame-dark`）。
#[test]
#[ignore]
fn shoot_blame() {
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let Some((mut app, root)) = git_app_in(&format!("git-blame-shot-{theme}"), false, dark)
        else {
            return;
        };
        blame_repo(&root);
        app.git_refresh();
        app.open_path(&root.join("src/server.rs"));
        app.run_action(Action::GitBlame);
        let at = app.editor.active().unwrap().text().line_start(1);
        app.select(at, at);
        app.run(winc_editor::Command::Insert(
            "// TODO：限制并发连接数\n".into(),
        ));
        paint(&mut app);
        // 鼠标停在第 7 行（「记录每个连接的来源」）上，提示已显示。
        let l = app.layout;
        let y = l.text.y + 6.5 * app.fonts.line_height;
        app.on_mouse_move(l.gutter.x + 40.0, y);
        app.tooltip_since = Some(std::time::Instant::now() - crate::theme::TOOLTIP_DELAY);
        shoot(&mut app, &format!("blame-{theme}"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
