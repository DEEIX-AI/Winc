//! 远程模块：经 `ssh` 连接其他主机，并把其上的目录作为工作区打开。
//!
//! 连接在后台线程上建立：启动系统的 `ssh`，在远端按需上传并启动服务进程（见
//! [`winc_backend::remote::Deploy`]），再取得目录在远端的真实路径。`ssh` 需要用户输入口令或
//! 确认主机密钥时，经 [`crate::askpass`] 以询问对话框提出。登录方式、主机密钥和其余设置
//! 都取自用户的 `ssh` 配置，这里不保存任何凭据。
//!
//! 模块关闭时命令不可用，不启动 `ssh`；关闭模块会断开已有的连接。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use winc_backend::Target;
use winc_backend::remote::{self, ConnectError, Connection, Deploy, Exec, Program};
use winc_editor::git::ASKPASS_ENDPOINT;
use winc_i18n::t;

use crate::app::git::question::Asker;
use crate::app::lifecycle::notice::NoticeAction;
use crate::app::{App, PromptKind, TextField, UserEvent};
use crate::askpass::{Listener, Question};

/// 指定服务进程所在目录的环境变量；未设置时在本程序所在的目录中查找。
const SERVER_DIR: &str = "WINC_SERVER_DIR";

/// 服务进程的文件名前缀，其后是它所运行的系统（写法见 [`Program::system`]）。
const SERVER_PREFIX: &str = "winc-server-";

/// 取得目录在远端的真实路径的脚本，参数是用户输入的目录。
///
/// 相对路径和以 `~/` 开头的路径相对于主目录，空的输入表示主目录；`pwd -P` 给出不经过符号
/// 链接的路径。
const RESOLVE: &str = concat!(
    r#"p=$1; case $p in ""|"~") p=. ;; "~/"*) p=${p#"~/"} ;; esac; "#,
    r#"cd && cd -- "$p" && pwd -P"#,
);

/// 一次连接的结果：要打开的目录，或失败的原因（已是界面语言的文字）。
type Outcome = Result<PathBuf, String>;

/// 远程模块的状态。
pub(crate) struct RemoteState {
    /// 正在连接的主机；没有连接在进行时为 `None`。
    connecting: Option<Target>,
    /// 后台连接的结果，由 [`App::poll_remote`] 取走。
    outcome: Arc<Mutex<Option<Outcome>>>,
    /// `ssh` 的输入请求。
    questions: (Sender<Question>, Receiver<Question>),
    /// 用户是否关闭了本次连接的询问对话框：随后的失败按取消提示。
    pub(in crate::app) declined: bool,
    /// 是否已提示过当前目录所在主机的连接中断：重新连接之前不重复提示。
    lost: bool,
}

impl Default for RemoteState {
    fn default() -> Self {
        Self {
            connecting: None,
            outcome: Arc::default(),
            questions: channel(),
            declined: false,
            lost: false,
        }
    }
}

/// 把用户输入的 `主机:目录` 拆成连接目标和目录；没有 `:` 时目录为空。主机不符合
/// [`Target`] 的要求时返回 `None`。
fn parse_destination(input: &str) -> Option<(Target, &str)> {
    let input = input.trim();
    let (host, folder) = input.split_once(':').unwrap_or((input, ""));
    Some((Target::new(host)?, folder.trim()))
}

/// `name` 是否为「系统-架构」的写法（如 `Linux-x86_64`）：目录中名字相近的其他文件不算
/// 服务进程。
fn is_system(name: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    name.split_once('-')
        .is_some_and(|(system, machine)| word(system) && word(machine))
}

/// 读取随本程序发布的各个系统的服务进程。
#[allow(
    clippy::disallowed_methods,
    reason = "只访问本机：服务进程在本程序所在的目录中"
)]
fn programs() -> Vec<Program> {
    let dir = std::env::var_os(SERVER_DIR).map(PathBuf::from).or_else(|| {
        let program = std::env::current_exe().ok()?;
        Some(program.parent()?.to_path_buf())
    });
    let Some(entries) = dir.and_then(|dir| std::fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    let mut programs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let system = name.to_str().and_then(|n| n.strip_prefix(SERVER_PREFIX));
        let Some(system) = system.filter(|system| is_system(system)) else {
            continue;
        };
        let Ok(content) = std::fs::read(entry.path()) else {
            continue;
        };
        programs.push(Program {
            system: system.to_owned(),
            // 摘要只用来区分不同版本的程序，作为远端的目录名。
            digest: format!("{:016x}", winc_core::hash::fnv(content.iter().copied())),
            content,
        });
    }
    programs
}

/// 连接意外中断时的通知。
type Closed = Arc<dyn Fn() + Send + Sync>;

/// 建立到 `target` 的连接并登记；`ssh` 的输入请求交给 `listener`，没有时不允许它提问。连接
/// 意外中断时调用 `closed`。
fn connect(
    target: &Target,
    listener: Option<&Listener>,
    closed: Option<&Closed>,
) -> Result<(), String> {
    let deploy = Deploy {
        dir: String::new(),
        programs: programs(),
    };
    if deploy.programs.is_empty() {
        return Err(t!("workspace-remote-no-server").to_owned());
    }
    let askpass = listener.and_then(Listener::askpass);
    let options = remote::Ssh {
        batch: askpass.is_none(),
        // 用户没有为该主机设置探测间隔时才添加，不覆盖用户的设置。
        keep_alive: !remote::keeps_alive(target),
    };
    let mut command: Command = deploy
        .ssh_command(target, options)
        .map_err(|e| e.to_string())?;
    if let Some(askpass) = askpass {
        // `SSH_ASKPASS_REQUIRE` 使 `ssh` 不依赖 `DISPLAY` 和控制终端。
        command
            .env("SSH_ASKPASS", &askpass.program)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ASKPASS_ENDPOINT, &askpass.endpoint);
    }
    let connection = Connection::deploy(command, &deploy).map_err(|error| match error {
        ConnectError::Unsupported(system) => {
            t!("workspace-remote-unsupported-system", system = system)
        }
        other => other.to_string(),
    })?;
    if let Some(closed) = closed.cloned() {
        connection.on_close(move || closed());
    }
    remote::register(target.clone(), connection);
    Ok(())
}

/// 返回 `folder` 在 `target` 上的真实路径所对应的虚拟路径；不是目录或无法进入时返回原因。
fn resolve(target: &Target, folder: &str) -> std::io::Result<Result<PathBuf, String>> {
    let home = winc_backend::virtual_path(target, b"/");
    let exec = Exec::new("sh")
        .arg("-c")
        .arg(RESOLVE)
        .arg("winc")
        .arg(folder);
    let output = remote::exec(&home, &exec, &|| false)?;
    if output.code != Some(0) {
        let reason = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Ok(Err(reason));
    }
    let path = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
    Ok(Ok(winc_backend::virtual_path(target, path)))
}

/// 连接 `target`（已有连接时沿用）并取得 `folder` 的虚拟路径。
fn open(
    target: &Target,
    folder: &str,
    listener: Option<&Listener>,
    closed: Option<&Closed>,
) -> Outcome {
    let existing = remote::is_connected(target);
    if !existing {
        connect(target, listener, closed)?;
    }
    match resolve(target, folder) {
        Ok(outcome) => outcome,
        // 沿用的连接已经中断：重新连接一次。
        Err(_) if existing => {
            connect(target, listener, closed)?;
            resolve(target, folder).map_err(|e| e.to_string())?
        }
        Err(error) => Err(error.to_string()),
    }
}

impl App {
    /// 远程模块是否启用。
    pub(crate) fn remote_enabled(&self) -> bool {
        self.config.remote_enabled.unwrap_or(false)
    }

    /// 打开输入条，输入要连接的主机和目录。
    pub(in crate::app) fn open_remote_prompt(&mut self) {
        self.open_prompt(PromptKind::Remote, TextField::default());
    }

    /// 连接输入条中的 `input`（`主机:目录`）并打开该目录。
    pub(in crate::app) fn remote_connect(&mut self, input: &str) {
        match parse_destination(input) {
            Some((target, folder)) => self.remote_open(target, folder.to_owned()),
            None => self.notify(t!("workspace-remote-invalid-host", input = input.trim())),
        }
    }

    /// 最近列表中的目录 `folder` 在远端主机上时，在后台连接（已有连接时沿用）并打开，返回真；
    /// 本机的目录返回假，由调用方按通常的方式打开。
    ///
    /// 远端的目录是否存在要向远端查询，不能在界面线程上判断。
    pub(in crate::app) fn reopen_remote(&mut self, folder: &Path) -> bool {
        let Some(location) = winc_backend::locate(folder) else {
            return false;
        };
        if self.remote_enabled() {
            let path = String::from_utf8_lossy(&location.path).into_owned();
            self.remote_open(location.target, path);
        } else {
            self.notify(t!("workspace-remote-disabled"));
        }
        true
    }

    /// 在后台连接 `target`，完成后打开其上的目录 `folder`；同时只进行一次连接。
    fn remote_open(&mut self, target: Target, folder: String) {
        if self.remote.connecting.is_some() {
            self.flash(t!("workspace-remote-busy"));
            return;
        }
        self.flash(t!("workspace-remote-connecting", host = target.as_str()));
        self.remote.connecting = Some(target.clone());
        self.remote.declined = false;
        let slot = Arc::clone(&self.remote.outcome);
        let wake = self.proxy.clone();
        let questions = self.remote.questions.0.clone();
        self.spawn_blocking_then(UserEvent::RemoteConnected, move || {
            // 端点只在连接期间存在，`listener` 在任务结束时被丢弃。
            let closed = wake.clone().map(|proxy| {
                let target = target.clone();
                let notify = move || {
                    let _ = proxy.send_event(UserEvent::RemoteClosed(target.clone()));
                };
                Arc::new(notify) as Closed
            });
            let listener = wake.and_then(|proxy| {
                Listener::bind(move |question| {
                    let _ = questions.send(question);
                    let _ = proxy.send_event(UserEvent::RemoteQuestion);
                })
            });
            let outcome = open(&target, &folder, listener.as_ref(), closed.as_ref());
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
        });
    }

    /// 处理后台连接的结果：成功时打开目录，失败时说明原因；用户取消输入时只提示已取消。
    pub(in crate::app) fn poll_remote(&mut self) {
        let outcome = self
            .remote
            .outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let (Some(outcome), Some(target)) = (outcome, self.remote.connecting.take()) else {
            return;
        };
        self.end_remote_questions();
        match outcome {
            Ok(folder) if self.remote_enabled() => {
                self.clear_flash();
                self.remote.lost = false;
                if self.editor.root() == Some(folder.as_path()) {
                    // 重新连接到已打开的目录：按窗口重新获得焦点处理，重读文件树和文档。
                    self.check_disk();
                } else {
                    self.open_folder(&folder);
                }
            }
            // 连接期间模块被关闭：连接已随之断开。
            Ok(_) => {}
            Err(_) if std::mem::take(&mut self.remote.declined) => {
                self.flash(t!("workspace-remote-canceled"));
            }
            Err(error) => self.notify(t!(
                "workspace-remote-failed",
                host = target.as_str(),
                error = error
            )),
        }
    }

    /// 处理 `ssh` 送达的输入请求；没有连接在进行时丢弃，askpass 程序按取消处理。
    pub(in crate::app) fn poll_remote_questions(&mut self) {
        while let Ok(question) = self.remote.questions.1.try_recv() {
            if self.remote.connecting.is_some() {
                self.open_question(Asker::Remote, question);
            }
        }
    }

    /// 到 `target` 的连接意外中断：打开的目录在该主机上时提示，并提供重新连接的按钮。
    pub(in crate::app) fn remote_closed(&mut self, target: &Target) {
        let root = self.editor.root().map(Path::to_path_buf);
        let Some(root) = root.filter(|root| {
            winc_backend::locate(root).is_some_and(|location| location.target == *target)
        }) else {
            return;
        };
        if self.remote.lost || self.remote.connecting.is_some() || !self.remote_enabled() {
            return;
        }
        self.remote.lost = true;
        self.notify_with(
            t!("workspace-remote-lost", host = target.as_str()),
            NoticeAction::Reconnect(root),
        );
    }

    /// 窗口重新获得焦点时调用：打开的目录所在主机的连接已不可用时提示（见
    /// [`App::remote_closed`]）。连接是进程内各窗口共用的，中断的通知只送到发起连接的窗口。
    pub(in crate::app) fn check_remote(&mut self) {
        let target = (self.editor.root())
            .and_then(winc_backend::locate)
            .map(|location| location.target);
        if let Some(target) = target.filter(|target| !remote::is_connected(target)) {
            self.remote_closed(&target);
        }
    }

    /// 关闭远程模块时调用：断开全部连接。
    pub(in crate::app) fn stop_remote(&mut self) {
        remote::disconnect_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_split_into_a_host_and_a_folder() {
        let parsed = |input| {
            parse_destination(input).map(|(target, folder)| (target.as_str().to_owned(), folder))
        };
        let host = "me@devbox".to_owned();
        assert_eq!(
            parsed("me@devbox:/srv/app"),
            Some((host.clone(), "/srv/app"))
        );
        assert_eq!(parsed("  me@devbox:~/x y  "), Some((host.clone(), "~/x y")));
        assert_eq!(parsed("me@devbox"), Some((host.clone(), "")));
        assert_eq!(parsed("me@devbox:"), Some((host, "")));
        // 主机之后的内容原样作为目录，其中的 `:` 不再拆分。
        assert_eq!(parsed("box:/a:b"), Some(("box".to_owned(), "/a:b")));
        // 不能作为 `ssh` 主机参数的输入被拒绝：选项、空白和 shell 的特殊字符。
        for input in [
            "",
            ":/srv",
            "-oProxyCommand=x:/",
            "a b:/",
            "a;b:/",
            "$(x):/",
        ] {
            assert_eq!(parsed(input), None, "{input:?}");
        }
    }

    #[test]
    fn only_files_named_after_a_system_are_server_programs() {
        for name in ["Linux-x86_64", "Linux-aarch64", "Darwin-arm64"] {
            assert!(is_system(name), "{name}");
        }
        // 构建目录中带哈希后缀的文件、调试信息和其他附属文件。
        for name in [
            "",
            "0a1b2c3d4e5f6071",
            "Linux-x86_64.d",
            "Linux-",
            "-x86_64",
            "a-b-c",
        ] {
            assert!(!is_system(name), "{name}");
        }
    }

    /// 经真实的 `ssh` 连接并取得主目录：需要环境变量 `WINC_TEST_SSH_TARGET`（免输入即可登录的
    /// 主机）和 `WINC_SERVER_DIR`（其中有该主机所用系统的服务进程）。
    #[test]
    #[ignore = "需要一台可以免输入登录的主机"]
    fn over_ssh() {
        let target = Target::new(&std::env::var("WINC_TEST_SSH_TARGET").unwrap()).unwrap();
        let home = open(&target, "~", None, None).unwrap();
        assert!(winc_backend::fs::is_dir(&home), "{}", home.display());
        let sub = open(&target, ".ssh", None, None).unwrap();
        assert_eq!(sub, home.join(".ssh"));
        let missing = open(&target, "/winc/no/such/folder", None, None).unwrap_err();
        assert!(missing.contains("/winc/no/such/folder"), "{missing}");
        remote::disconnect(&target);
    }
}
