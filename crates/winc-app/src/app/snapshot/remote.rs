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
