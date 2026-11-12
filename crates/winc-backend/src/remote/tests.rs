//! 连接的登记与取用、界面线程的限制，以及 `ssh` 命令的组成。

use super::*;

#[test]
fn unknown_targets_are_not_connected_and_blocking_can_be_forbidden() {
    let target = Target::new("no-such-connection").unwrap();
    let error = connection(&target).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
    disconnect(&target);

    // 限制只作用于调用它的线程。
    let forbidden = std::thread::spawn(move || {
        forbid_blocking();
        connection(&target).err().unwrap().kind()
    });
    assert_eq!(forbidden.join().unwrap(), io::ErrorKind::WouldBlock);
    let target = Target::new("no-such-connection").unwrap();
    let error = connection(&target).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotConnected);
}

#[test]
fn the_ssh_command_passes_the_program_as_one_quoted_word() {
    let target = Target::new("alice@devbox").unwrap();
    let command = ssh_command(
        &target,
        "/home/alice/.winc-server/it's here/winc-server",
        Ssh::default(),
    );
    let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(command.get_program(), "ssh");
    assert_eq!(
        args,
        [
            "-T",
            "-o",
            "RemoteCommand=none",
            "alice@devbox",
            "--",
            r"'/home/alice/.winc-server/it'\''s here/winc-server'",
        ]
    );
    // 端口由 `-p` 给出，主机参数中不含端口。
    let with_port = Target::new("alice@devbox:22022").unwrap();
    let command = ssh_command(&with_port, "server", Ssh::default());
    let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(args[3..], ["-p", "22022", "alice@devbox", "--", "'server'"]);
    // 不向用户提问的选项写在主机之前：主机之后的参数属于远端的命令。
    let options = Ssh {
        batch: true,
        keep_alive: true,
        connect_timeout: true,
        share: false,
    };
    let command = ssh_command(&target, "server", options);
    let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(
        args,
        [
            "-T",
            "-o",
            "RemoteCommand=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "ServerAliveInterval=60",
            "-o",
            "ConnectTimeout=15",
            "alice@devbox",
            "--",
            "'server'"
        ]
    );
}

#[test]
fn a_registered_key_file_is_passed_to_ssh_as_one_argument() {
    let target = Target::new("alice@keyed-host:2200").unwrap();
    let args = || -> Vec<String> {
        let command = ssh_command(&target, "server", Ssh::default());
        (command.get_args())
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect()
    };
    set_identity(&target, Some("~/.ssh/my key"));
    assert_eq!(identity(&target).as_deref(), Some("~/.ssh/my key"));
    assert_eq!(
        args()[3..],
        [
            "-i",
            "~/.ssh/my key",
            "-p",
            "2200",
            "alice@keyed-host",
            "--",
            "'server'"
        ]
    );
    // 空的路径去掉登记。
    set_identity(&target, Some(""));
    assert_eq!(identity(&target), None);
    assert_eq!(
        args()[3..],
        ["-p", "2200", "alice@keyed-host", "--", "'server'"]
    );
}

#[test]
fn only_settings_the_user_left_unset_are_filled_in() {
    let unset = "user alice\nserveraliveinterval 0\nconnecttimeout none\n";
    let filled = Ssh {
        batch: false,
        keep_alive: true,
        connect_timeout: true,
        share: false,
    };
    assert_eq!(Ssh::missing(unset), filled);
    // 连接复用：用户没有设置 `ControlMaster`、`ControlPath` 时才补。
    let plain = format!("{unset}controlmaster false\ncontrolpersist no\n");
    assert!(Ssh::missing(&plain).share);
    let own = format!("{plain}controlpath /home/alice/.ssh/cm-%C\n");
    assert!(!Ssh::missing(&own).share);
    assert!(!Ssh::missing("controlmaster auto\n").share);
    let set = "ServerAliveInterval 30\nConnectTimeout 5\n";
    assert_eq!(Ssh::missing(set), Ssh::default());
    // 读不到的项不补：不能确定用户没有设置。
    assert_eq!(Ssh::missing(""), Ssh::default());
}

#[cfg(unix)]
#[test]
fn a_command_that_is_not_the_server_fails_the_handshake() {
    // `true` 立即退出，不应答：握手读到输入结束。
    let error = Connection::spawn(Command::new("true")).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(Connection::spawn(Command::new("winc-no-such-program")).is_err());
}
