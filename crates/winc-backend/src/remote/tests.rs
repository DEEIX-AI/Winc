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
            "alice@devbox",
            "--",
            "'server'"
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_command_that_is_not_the_server_fails_the_handshake() {
    // `true` 立即退出，不应答：握手读到输入结束。
    let error = Connection::spawn(Command::new("true")).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(Connection::spawn(Command::new("winc-no-such-program")).is_err());
}
