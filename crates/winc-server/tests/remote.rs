//! 端到端：本机的文件操作经连接到达服务进程，结果与直接访问本机文件一致。
//!
//! 服务进程直接作为子进程启动，不经过 `ssh`；经 `ssh` 的测试需要一台可登录的主机，默认不运行
//! （见 [`over_ssh`]）。

#![cfg(unix)]
#![allow(
    clippy::disallowed_methods,
    reason = "测试只在 Unix 上运行，直接取临时目录的真实路径"
)]

use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use winc_backend::remote::{self, Connection};
use winc_backend::{Target, fs, virtual_path};

/// 启动服务进程并登记为到 `name` 的连接，返回连接目标。
fn connect(name: &str) -> Target {
    let target = Target::new(name).unwrap();
    let server = Command::new(env!("CARGO_BIN_EXE_winc-server"));
    remote::register(target.clone(), Connection::spawn(server).unwrap());
    target
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("winc-server-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 临时目录可能经过符号链接，取真实路径作为远端路径。
    dir.canonicalize().unwrap()
}

/// 本机路径 `path` 在连接 `target` 上对应的虚拟路径。
fn through(target: &Target, path: &Path) -> PathBuf {
    virtual_path(target, path.as_os_str().as_bytes())
}

#[test]
fn files_read_through_a_connection_match_the_local_ones() {
    let target = connect("read-files");
    let dir = scratch("read");
    std::fs::write(dir.join("small.txt"), "héllo\n").unwrap();
    // 超过一次读取的上限（1 MiB），分多次取回；内容逐字节不同，错位能被发现。
    let large: Vec<u8> = (0..3_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join("large.bin"), &large).unwrap();
    std::fs::write(dir.join("empty"), "").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::os::unix::fs::symlink("small.txt", dir.join("link")).unwrap();
    // 不是有效 UTF-8 的文件名：macOS 的文件系统不接受，只在其他系统上检查。
    let odd = std::ffi::OsStr::from_bytes(b"caf\xE9.txt");
    let odd_names = !cfg!(target_os = "macos");
    if odd_names {
        std::fs::write(dir.join(odd), "x").unwrap();
    }
    let far = through(&target, &dir);

    for name in ["small.txt", "large.bin", "empty", "sub", "link"] {
        let (local, remote) = (dir.join(name), far.join(name));
        assert_eq!(
            fs::metadata(&remote).unwrap(),
            fs::metadata(&local).unwrap()
        );
        assert_eq!(
            fs::symlink_metadata(&remote).unwrap(),
            fs::symlink_metadata(&local).unwrap(),
            "{name}"
        );
    }
    assert_eq!(
        fs::read(far.join("small.txt")).unwrap(),
        "héllo\n".as_bytes()
    );
    assert_eq!(fs::read_to_string(far.join("link")).unwrap(), "héllo\n");
    assert_eq!(fs::read(far.join("large.bin")).unwrap(), large);
    assert!(fs::read(far.join("empty")).unwrap().is_empty());
    assert!(fs::is_dir(far.join("sub")) && fs::is_file(far.join("link")));
    assert!(fs::is_symlink(far.join("link")) && fs::exists(&far));

    // 列目录：名字、种类与本机一致，路径仍是该连接上的虚拟路径；非 UTF-8 的名字原样往返。
    let list = |dir: &Path| {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .map(|entry| (entry.name.clone(), entry.kind, entry.path()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    };
    let (near, remote) = (list(&dir), list(&far));
    assert_eq!(remote.len(), 5 + usize::from(odd_names));
    for ((name, kind, local), (far_name, far_kind, far_path)) in near.iter().zip(&remote) {
        assert_eq!((name, kind), (far_name, far_kind));
        assert_eq!(*far_path, through(&target, local));
        assert_eq!(
            winc_backend::locate(far_path).unwrap().path,
            local.as_os_str().as_bytes()
        );
    }
    if odd_names {
        assert_eq!(fs::read(far.join(odd)).unwrap(), b"x");
    }

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn remote_errors_keep_their_kind() {
    let target = connect("errors");
    let dir = scratch("errors");
    std::fs::write(dir.join("file"), "x").unwrap();
    let far = through(&target, &dir);

    let missing = fs::metadata(far.join("missing")).unwrap_err();
    assert!(fs::is_absent(&missing), "{missing}");
    assert!(fs::is_absent(
        &fs::metadata(far.join("file/below")).unwrap_err()
    ));
    assert!(fs::is_absent(&fs::read(far.join("missing")).unwrap_err()));
    assert!(fs::read_dir(far.join("file")).is_err());
    assert!(fs::read(&far).is_err(), "目录不能当作文件读取");
    // 非 UTF-8 的内容按文本读取时是数据错误，不是连接错误。
    std::fs::write(dir.join("binary"), [0xFF, 0xFE]).unwrap();
    let error = fs::read_to_string(far.join("binary")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    // 出错之后连接仍然可用。
    assert_eq!(fs::read(far.join("file")).unwrap(), b"x");

    remote::disconnect(&target);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn connections_are_per_target_and_refuse_threads_that_must_not_wait() {
    let target = connect("per-target");
    let dir = scratch("per-target");
    std::fs::write(dir.join("file"), "x").unwrap();
    let file = through(&target, &dir.join("file"));
    assert!(fs::is_file(&file));

    // 另一个目标没有连接；同一路径在不同目标下互不相通。
    let other = through(&Target::new("someone-else").unwrap(), &dir.join("file"));
    let error = fs::metadata(&other).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);

    // 不允许等待的线程：立即返回，不发出请求；本机路径不受影响。
    let (far, near) = (file.clone(), dir.join("file"));
    let kinds = std::thread::spawn(move || {
        remote::forbid_blocking();
        (
            fs::metadata(&far).unwrap_err().kind(),
            fs::read(&far).unwrap_err().kind(),
            fs::is_file(&far),
            fs::is_file(&near),
        )
    });
    assert_eq!(
        kinds.join().unwrap(),
        (
            io::ErrorKind::WouldBlock,
            io::ErrorKind::WouldBlock,
            false,
            true
        )
    );

    // 断开之后不再可用。
    remote::disconnect(&target);
    let error = fs::metadata(&file).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 经真实的 `ssh` 连接：需要环境变量 `WINC_TEST_SSH_TARGET`（免输入即可登录的主机）和
/// `WINC_TEST_SSH_SERVER`（该主机上服务进程的路径）。
#[test]
#[ignore = "需要一台可以免输入登录、已放好服务进程的主机"]
fn over_ssh() {
    let host = std::env::var("WINC_TEST_SSH_TARGET").unwrap();
    let server = std::env::var("WINC_TEST_SSH_SERVER").unwrap();
    let target = Target::new(&host).unwrap();
    let mut command = remote::ssh_command(&target, &server);
    command.args(["-o", "BatchMode=yes"]);
    remote::register(target.clone(), Connection::spawn(command).unwrap());

    let root = virtual_path(&target, b"/");
    assert!(fs::is_dir(&root));
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().name)
        .collect();
    assert!(names.iter().any(|name| name == "etc"), "{names:?}");
    let passwd = fs::read_to_string(root.join("etc/passwd")).unwrap();
    assert!(passwd.contains("root:"), "{passwd}");
    let absent = fs::metadata(root.join("no/such/file")).unwrap_err();
    assert!(fs::is_absent(&absent));
    remote::disconnect(&target);
}
