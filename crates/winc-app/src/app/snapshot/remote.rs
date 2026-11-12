//! 远程模块的测试：模块开关、经连接打开目录和文件、失败的提示，以及最近列表中的远端目录。
//!
//! 服务端在本进程内运行，不经过 `ssh`；各项检查放在同一个测试中，因为关闭模块会断开进程内
//! 的全部连接。

use std::os::unix::ffi::OsStrExt;

use winc_backend::remote;

use super::*;
use crate::keymap::Action;

/// 在本进程内启动服务端并登记为到 `name` 的连接：该目标的虚拟路径指向本机的同名路径。
fn loopback(name: &str) -> winc_backend::Target {
    let (requests, to_server) = std::io::pipe().unwrap();
    let (replies, from_server) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let _ = remote::serve(requests, from_server);
    });
    let target = winc_backend::Target::new(name).unwrap();
    let connection = remote::Connection::over(replies, to_server).unwrap();
    remote::register(target.clone(), connection);
    target
}

#[test]
fn a_folder_on_a_connected_host_opens_as_the_workspace() {
    let dir = sample_folder("remote-open").canonicalize().unwrap();
    std::os::unix::fs::symlink(&dir, dir.with_extension("link")).unwrap();
    let repository = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .is_ok_and(|status| status.success());
    let mut app = blank_app(false);
    app.config.git_enabled = Some(true);
    // 测试线程相当于界面线程：不允许等待远端的应答，后台任务就地执行时除外。
    remote::forbid_blocking();

    // 模块默认关闭：命令不可用，按下也不打开输入条。
    assert!(!app.action_available(Action::RemoteConnect));
    assert!(!app.run_action(Action::RemoteConnect));
    assert!(app.goto_input.is_none());
    app.config.remote_enabled = Some(true);
    assert!(app.run_action(Action::RemoteConnect));
    assert_eq!(app.prompt_kind, crate::app::PromptKind::Remote);
    app.close_prompt();

    // 不是有效主机的输入不会交给 `ssh`。
    app.remote_connect("-oProxyCommand=touch /tmp/x:/");
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("不是有效的主机"), "{notice}");

    // 经符号链接给出的目录按远端的真实路径打开，文件树的内容经连接列出。
    let target = loopback("app-remote-open");
    let link = dir.with_extension("link");
    app.remote_connect(&format!("app-remote-open:{}", link.display()));
    let root = winc_backend::virtual_path(&target, dir.as_os_str().as_bytes());
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    let names: Vec<_> = (app.editor.workspace.as_ref().unwrap().rows().iter())
        .map(|entry| entry.name.clone())
        .collect();
    assert!(names.iter().any(|name| name == "src"), "{names:?}");
    assert!(names.iter().any(|name| name == "a.txt"), "{names:?}");

    // 仓库由后台查找，状态经连接上的 `git` 读取：尚未提交的文件都是未跟踪的。
    if repository {
        assert_eq!(app.git.root.as_deref(), Some(root.as_path()));
        assert!(app.git.status.is_some(), "没有读到仓库的状态");
    }

    // 其中的文件经连接读取。
    app.open_path(&root.join("src/main.rs"));
    let doc = app.editor.active().unwrap();
    assert_eq!(
        doc.path.as_deref(),
        Some(root.join("src/main.rs").as_path())
    );
    assert!(doc.text().rope().unwrap().to_string().contains("println!"));

    // 展开子目录：内容由后台读取后换入。
    let src = row_of(&app, "src");
    app.set_dir_expanded(src, Some(true));
    assert_eq!(row_of(&app, "main.rs"), src + 1);

    // 编辑后保存：内容经连接写到远端。
    let doc = app.editor.active_mut().unwrap();
    doc.apply(winc_core::edit::replace_selection(
        doc.text().rope().unwrap(),
        winc_core::Selection::point(0),
        "// remote\n",
        winc_core::EditKind::Other,
    ));
    assert!(app.run_action(Action::Save));
    let saved = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert!(saved.starts_with("// remote\n"), "{saved}");
    assert!(!app.editor.active().unwrap().is_dirty());

    // 其他程序在远端新建了文件，窗口重新获得焦点：文件树重读后出现，已展开的目录保持展开。
    std::fs::write(dir.join("src/new.rs"), "").unwrap();
    app.check_disk();
    assert_eq!(row_of(&app, "new.rs"), src + 2);
    assert_eq!(row_of(&app, "main.rs"), src + 1);

    // 文件树中的操作在后台执行，完成后文件树随之更新。新建：
    let new_input = |dir: &Path, name: &str| crate::app::workspace::tree::NameInput {
        dir: dir.to_path_buf(),
        kind: winc_editor::EntryKind::File,
        index: 0,
        depth: 0,
        field: {
            let mut field = crate::app::TextField::default();
            field.set_text(name);
            field
        },
        error: None,
        rename: None,
        contents: None,
    };
    app.name_input = Some(new_input(&root.join("src"), "made.rs"));
    app.submit_name_input();
    assert!(dir.join("src/made.rs").is_file());
    assert!(app.name_input.is_none());
    let made = row_of(&app, "made.rs");
    assert_eq!(app.editor.workspace.as_ref().unwrap().selected, Some(made));
    // 同名的再建一次：原因显示在输入行上。
    app.name_input = Some(new_input(&root.join("src"), "made.rs"));
    app.submit_name_input();
    assert!(
        app.name_input
            .as_ref()
            .is_some_and(|entry| entry.error.is_some())
    );
    app.close_name_input();

    // 改名：已打开的文档换成新路径。
    let mut rename = new_input(&root.join("src"), "renamed.rs");
    rename.rename = Some((root.join("src/made.rs"), false));
    app.submit_rename(rename);
    assert!(dir.join("src/renamed.rs").is_file() && !dir.join("src/made.rs").exists());
    row_of(&app, "renamed.rs");
    assert!((app.editor.documents.iter()).any(|d| d.path == Some(root.join("src/renamed.rs"))));

    // 移动到根目录，再删除：远端没有废纸篓，直接删除。
    app.move_entries(&[root.join("src/renamed.rs")], &root);
    assert!(dir.join("renamed.rs").is_file() && !dir.join("src/renamed.rs").exists());
    row_of(&app, "renamed.rs");
    std::fs::create_dir_all(dir.join("old/deep")).unwrap();
    std::fs::write(dir.join("old/deep/x"), "").unwrap();
    app.delete_entries(&[root.join("renamed.rs"), root.join("old")]);
    assert!(!dir.join("renamed.rs").exists() && !dir.join("old").exists());
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);

    // 目录不存在：说明原因，已打开的目录不变。
    app.remote_connect("app-remote-open:/winc/no/such/folder");
    let notice = app.notice.text.take().unwrap();
    assert!(notice.starts_with("无法连接 app-remote-open："), "{notice}");
    assert_eq!(app.editor.root(), Some(root.as_path()));

    // 最近列表中远端的目录经同一流程打开，不在界面线程上判断它是否存在；本机的目录不经此处。
    assert!(app.state.recent.contains(&root));
    assert!(app.reopen_remote(&root));
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    assert_eq!(app.editor.root(), Some(root.as_path()));
    assert!(!app.reopen_remote(&dir));

    // 连接中断（这里由本程序断开来模拟）：窗口重新获得焦点时提示一次，按钮重新连接。
    remote::disconnect(&target);
    app.check_disk();
    let notice = app.notice.text.clone().unwrap();
    assert!(notice.contains("连接已断开"), "{notice}");
    assert_eq!(
        app.notice.action,
        Some(crate::app::NoticeAction::Reconnect(root.clone()))
    );
    // 目录仍然打开，文件树保持原样。
    assert_eq!(app.editor.root(), Some(root.as_path()));
    row_of(&app, "src");
    app.notice.text = None;
    app.check_disk();
    assert!(app.notice.text.is_none());
    let target = loopback("app-remote-open");
    app.run_notice_action();
    assert!(app.notice.text.is_none(), "{:?}", app.notice.text);
    std::fs::write(dir.join("after.txt"), "").unwrap();
    app.check_disk();
    row_of(&app, "after.txt");

    // 关闭模块：连接断开。此后最近列表中的该目录不会被当作已删除的目录移除，而是提示原因。
    app.config.remote_enabled = Some(false);
    app.stop_remote();
    assert!(!remote::is_connected(&target));
    assert!(app.reopen_remote(&root));
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("远程模块未启用"), "{notice}");

    // 重新启用后需要重新连接；测试环境中没有可上传的服务程序，在启动 `ssh` 之前失败。
    app.config.remote_enabled = Some(true);
    assert!(app.reopen_remote(&root));
    let notice = app.notice.text.take().unwrap();
    assert!(notice.contains("缺少在远端运行的服务程序"), "{notice}");
    assert!(app.state.recent.contains(&root));
}
