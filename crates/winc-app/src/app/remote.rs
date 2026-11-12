//! 远程模块：经 `ssh` 连接其他主机，并把其上的目录作为工作区打开。
//!
//! 连接在后台线程上建立：启动系统的 `ssh`，在远端按需上传并启动服务进程（见
//! [`winc_backend::remote::Deploy`]），再取得目录在远端的真实路径。`ssh` 需要用户输入口令或
//! 确认主机密钥时，经 [`crate::askpass`] 以询问对话框提出。登录方式、主机密钥和其余设置
//! 都取自用户的 `ssh` 配置，这里不保存任何凭据。
//!
//! 模块关闭时命令不可用，不启动 `ssh`；关闭模块会断开已有的连接。

pub(crate) mod picker;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use winc_backend::Target;
use winc_backend::remote::{self, ConnectError, Connection, Deploy, Exec, Program};
use winc_editor::git::ASKPASS_ENDPOINT;
use winc_i18n::t;

use crate::app::git::form::{FormDialog, FormKind, FormPlan};
use crate::app::git::question::Asker;
use crate::app::lifecycle::notice::NoticeAction;
use crate::app::{App, TextField, UserEvent};
use crate::askpass::{Listener, Question};
use crate::layout::DialogPart;

/// 指定服务进程所在目录的环境变量；未设置时在安装位置中查找（见 [`server_dirs`]）。
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

/// 连接成功之后做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum After {
    /// 打开取得的目录。
    Open,
    /// 以取得的目录为起点，在面板中浏览并选择要打开的目录。
    Browse,
}

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
    /// 中止正在进行的连接；没有连接在进行时为 `None`。
    abort: Option<remote::Abort>,
    /// 正在进行的连接成功之后做什么。
    after: After,
    /// 连接对话框中填写的主机和端口；不是经对话框连接时为 `None`。对话框暂时让位给询问、
    /// 或连接失败后重新显示时按它恢复。
    form: Option<(String, String)>,
    /// 接下来对连接对话框的关闭是程序进行的（进入下一步、让位给询问），不是用户取消。
    stepping: bool,
    /// 选择面板在后台读好的目录。
    pub(in crate::app) listed: picker::Listings,
}

impl Default for RemoteState {
    fn default() -> Self {
        Self {
            connecting: None,
            outcome: Arc::default(),
            questions: channel(),
            declined: false,
            lost: false,
            abort: None,
            after: After::Open,
            form: None,
            stepping: false,
            listed: picker::Listings::default(),
        }
    }
}

/// 项目名菜单中列出的主机：菜单项按下标引用，打开菜单时更新（见 [`refresh_hosts`]）。
static HOSTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// 列出过的主机名。菜单项的文字要求 `'static`：每个不同的名字只保留一份，数量以用户配置过
/// 的主机为限。
static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// 返回项目名菜单中列出的主机，顺序同用户的 `ssh` 配置。
pub(crate) fn hosts() -> Vec<&'static str> {
    HOSTS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// 重新读取用户的 `ssh` 配置（`~/.ssh/config`）中的主机，返回个数。
#[allow(
    clippy::disallowed_methods,
    reason = "只访问本机：用户主目录中的 ssh 配置"
)]
pub(crate) fn refresh_hosts() -> usize {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let config = home.map(|home| PathBuf::from(home).join(".ssh/config"));
    let text = config.and_then(|path| std::fs::read_to_string(path).ok());
    let found = config_hosts(text.as_deref().unwrap_or_default());
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    let listed: Vec<&'static str> = (found.into_iter())
        .map(|host| match names.iter().find(|known| **known == host) {
            Some(known) => *known,
            None => {
                let kept: &'static str = host.leak();
                names.push(kept);
                kept
            }
        })
        .collect();
    let count = listed.len();
    *HOSTS.lock().unwrap_or_else(PoisonError::into_inner) = listed;
    count
}

/// 返回 `ssh` 配置 `text` 中 `Host` 行列出的主机，按出现的顺序，不重复。
///
/// 只取可以直接连接的名字：带通配符（`*`、`?`）和取反（`!`）的是匹配规则，不是主机；不符合
/// [`Target`] 要求的名字也略过。`Include` 引入的文件不读取。
fn config_hosts(text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in text.lines() {
        // 关键字与参数之间是空白，或一个 `=`（两侧可有空白）。
        let line = line.trim();
        let split = line.find(|c: char| c.is_whitespace() || c == '=');
        let (keyword, rest) = line.split_at(split.unwrap_or(line.len()));
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        let rest = rest.trim_start().strip_prefix('=').unwrap_or(rest);
        // 参数以空白分隔，含空白的参数写在双引号内。
        let mut quoted = false;
        for name in rest.split(|c: char| {
            quoted ^= c == '"';
            !quoted && c.is_whitespace()
        }) {
            let name = name.trim_matches('"');
            let pattern = name.contains(['*', '?', '!']);
            if !pattern && Target::new(name).is_some() && !hosts.iter().any(|h| h == name) {
                hosts.push(name.to_owned());
            }
        }
    }
    hosts
}

/// 连接对话框「正在连接」时 `FormDialog::running` 的取值：它不是 Git 操作的编号，只表示
/// 按钮暂不可用。
const CONNECTING: u64 = u64::MAX;

/// 把用户输入的 `主机`、`主机:端口`、`主机:目录` 或 `主机:端口:目录` 拆成连接目标和目录；没有
/// 目录时目录为空。主机之后全是数字的一段是端口。主机或端口不符合 [`Target`] 的要求时返回
/// `None`。
fn parse_destination(input: &str) -> Option<(Target, &str)> {
    let input = input.trim();
    let Some((host, rest)) = input.split_once(':') else {
        return Some((Target::new(input)?, ""));
    };
    let (port, folder) = match rest.split_once(':') {
        Some((port, folder)) if is_port(port) => (Some(port), folder),
        None if is_port(rest) => (Some(rest), ""),
        _ => (None, rest),
    };
    let target = match port {
        Some(port) => Target::new(&format!("{host}:{port}"))?,
        None => Target::new(host)?,
    };
    Some((target, folder.trim()))
}

/// `text` 是否像端口：非空，全是数字。数值是否有效由 [`Target`] 判断。
fn is_port(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
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

/// 返回可能放有服务进程的目录，按查找的顺序。
///
/// 环境变量 [`SERVER_DIR`] 指定时只看它。否则是本程序所在的目录（Windows 的安装目录和便携版、
/// 开发时的构建目录），以及各系统安装包中放置数据文件的位置：macOS 应用包的 `Resources`，
/// Linux 上与 `bin` 并列的 `lib/winc`。
fn server_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os(SERVER_DIR) {
        return vec![PathBuf::from(dir)];
    }
    let program = std::env::current_exe().ok();
    let Some(beside) = program.as_deref().and_then(Path::parent) else {
        return Vec::new();
    };
    let mut dirs = vec![beside.to_path_buf()];
    if let Some(prefix) = beside.parent() {
        if cfg!(target_os = "macos") {
            dirs.push(prefix.join("Resources"));
        } else if cfg!(unix) {
            dirs.push(prefix.join("lib/winc"));
        }
    }
    dirs
}

/// 读取随本程序发布的各个系统的服务进程；同一个系统的程序出现在多个目录中时取先找到的。
#[allow(
    clippy::disallowed_methods,
    reason = "只访问本机：服务进程在本程序的安装位置中"
)]
fn programs() -> Vec<Program> {
    let entries = server_dirs()
        .into_iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten();
    let mut programs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let system = name.to_str().and_then(|n| n.strip_prefix(SERVER_PREFIX));
        let Some(system) = system.filter(|system| is_system(system)) else {
            continue;
        };
        if programs
            .iter()
            .any(|known: &Program| known.system == system)
        {
            continue;
        }
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
    abort: &remote::Abort,
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
    let connection = Connection::deploy(command, &deploy, abort).map_err(|error| match error {
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
    abort: &remote::Abort,
) -> Outcome {
    let existing = remote::is_connected(target);
    if !existing {
        connect(target, listener, closed, abort)?;
    }
    match resolve(target, folder) {
        Ok(outcome) => outcome,
        // 沿用的连接已经中断：重新连接一次。
        Err(_) if existing => {
            connect(target, listener, closed, abort)?;
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

    /// 连接主机 `host`（项目名菜单中列出的主机），连接后在面板中浏览其上的目录。
    pub(in crate::app) fn browse_remote_host(&mut self, host: &str) {
        self.open_connect_dialog(host, "", true);
    }

    /// 打开连接远端主机的对话框，填好主机 `host` 和端口 `port`（可以为空）；`start` 为真且
    /// 填写的内容有效时立即开始连接。
    ///
    /// 连接期间对话框一直在：按钮显示「正在连接…」，失败的原因写在对话框里，改了再连；成功
    /// 后进入选择目录的面板。取消回到主机面板。
    pub(in crate::app) fn open_connect_dialog(&mut self, host: &str, port: &str, start: bool) {
        self.remote.form = Some((host.to_owned(), port.to_owned()));
        // 已有的对话框（例如上一次的）由这一个替换，不算用户取消。
        if matches!(
            self.form_dialog.as_ref().map(|d| &d.kind),
            Some(FormKind::Connect)
        ) {
            self.remote.stepping = true;
            self.close_form_dialog();
        }
        let mut name = TextField::default();
        name.insert(host);
        self.show_form_dialog(FormKind::Connect, name, DialogPart::Branch);
        if let Some(dialog) = &mut self.form_dialog {
            dialog.second = TextField::with_filter(|c| c.is_ascii_digit());
            dialog.second.insert(port);
        }
        if start {
            self.submit_form_dialog();
        }
    }

    /// 连接对话框按当前填写的内容能不能连接，以及主按钮上的字。
    pub(in crate::app) fn connect_plan(&self, dialog: &FormDialog) -> FormPlan {
        let (host, port) = (dialog.name.text().trim(), dialog.second.text().trim());
        let problem = if !host.is_empty() && Target::with_port(host, None).is_none() {
            Some(t!("workspace-remote-dialog-invalid-host"))
        } else if !port.is_empty() && Target::with_port("host", port.parse().ok()).is_none() {
            Some(t!("workspace-remote-dialog-invalid-port"))
        } else {
            None
        };
        FormPlan {
            enabled: !host.is_empty() && problem.is_none(),
            problem: problem.map(str::to_owned),
            confirm: match dialog.running {
                Some(_) => t!("workspace-remote-dialog-connecting"),
                None => t!("workspace-remote-dialog-connect"),
            },
            base: String::new(),
            base_enabled: false,
            dir: None,
        }
    }

    /// 提交连接对话框：开始连接，对话框留着显示进度。
    pub(in crate::app) fn submit_connect(&mut self) {
        let Some(dialog) = &mut self.form_dialog else {
            return;
        };
        let (host, port) = (
            dialog.name.text().trim().to_owned(),
            dialog.second.text().trim().to_owned(),
        );
        let Some(target) = Target::with_port(&host, port.parse().ok().filter(|_| !port.is_empty()))
        else {
            return;
        };
        dialog.running = Some(CONNECTING);
        dialog.error = None;
        self.remote.form = Some((host, port));
        self.remote_open(target, String::new(), After::Browse);
        self.request_redraw();
    }

    /// 连接对话框即将被程序关闭（进入下一步、让位给询问）：这次关闭不算用户取消。
    pub(in crate::app) fn connect_dialog_steps_aside(&mut self) {
        let shown = self.form_dialog.as_ref().map(|dialog| &dialog.kind);
        self.remote.stepping = matches!(shown, Some(FormKind::Connect));
    }

    /// 连接对话框关闭时调用。用户取消（关闭按钮、`Esc`、「取消」）时中止正在进行的连接，回到
    /// 主机面板；程序自己关闭时不做任何事。
    pub(in crate::app) fn connect_dialog_closed(&mut self) {
        if std::mem::take(&mut self.remote.stepping) {
            return;
        }
        self.remote.form = None;
        self.cancel_remote();
        self.open_remote_picker();
    }

    /// 询问答复之后回到连接对话框：连接仍在进行时按「正在连接」显示。
    pub(in crate::app) fn restore_connect_dialog(&mut self) {
        let Some((host, port)) = self.remote.form.clone() else {
            return;
        };
        if self.remote.connecting.is_none() {
            return;
        }
        self.open_connect_dialog(&host, &port, false);
        if let Some(dialog) = &mut self.form_dialog {
            dialog.running = Some(CONNECTING);
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
            self.remote_open(location.target, path, After::Open);
        } else {
            self.notify(t!("workspace-remote-disabled"));
        }
        true
    }

    /// 在后台连接 `target`（已有连接时沿用）并取得其上目录 `folder` 的真实路径（空表示主目录），
    /// 完成后按 `after` 打开它或从它开始浏览；同时只进行一次连接。
    fn remote_open(&mut self, target: Target, folder: String, after: After) {
        // 提示留在右上角并带「取消」：登录可能在等待网络，没有时限。
        if let Some(busy) = &self.remote.connecting {
            let message = t!("workspace-remote-connecting", host = busy.as_str());
            self.notify_pinned(message, NoticeAction::CancelConnect);
            return;
        }
        // 经对话框连接时进度和取消都在对话框里，不另外提示。
        if self.remote.form.is_none() {
            let message = t!("workspace-remote-connecting", host = target.as_str());
            self.notify_pinned(message, NoticeAction::CancelConnect);
        }
        let abort = remote::Abort::default();
        self.remote.abort = Some(abort.clone());
        self.remote.connecting = Some(target.clone());
        self.remote.after = after;
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
            let outcome = open(&target, &folder, listener.as_ref(), closed.as_ref(), &abort);
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
        self.remote.abort = None;
        // 连接的提示到此为止；它的按钮已经没有可取消的对象。
        if self.notice.action == Some(NoticeAction::CancelConnect) {
            self.dismiss_notice();
        }
        // 经对话框连接：失败的原因写在对话框里（用户取消了询问时不写），改了可以再连。
        if let (Err(error), Some((host, port))) = (&outcome, self.remote.form.clone()) {
            let declined = std::mem::take(&mut self.remote.declined);
            self.open_connect_dialog(&host, &port, false);
            if let Some(dialog) = &mut self.form_dialog {
                dialog.error = (!declined).then(|| error.clone());
            }
            return;
        }
        match outcome {
            Ok(folder) if self.remote_enabled() && self.remote.after == After::Browse => {
                // 下一步：关闭连接对话框，在面板中选择目录。
                self.remote.form = None;
                self.connect_dialog_steps_aside();
                if self.remote.stepping {
                    self.close_form_dialog();
                }
                let home = winc_backend::native_lossy(&folder).into_owned();
                self.open_remote_browser(target, home);
            }
            Ok(folder) if self.remote_enabled() => {
                self.remote.lost = false;
                if self.editor.root() == Some(folder.as_path()) {
                    // 重新连接到已打开的目录：按窗口重新获得焦点处理，重读文件树和文档。
                    self.check_disk();
                } else {
                    self.open_folder(&folder);
                }
            }
            // 连接期间模块被关闭：关闭时这条连接尚未登记，在这里断开。
            Ok(_) => remote::disconnect(&target),
            Err(_) if std::mem::take(&mut self.remote.declined) => {
                self.flash(t!("workspace-remote-canceled"));
            }
            Err(error) => {
                // 重新连接失败后仍是断开的状态：下次获得焦点时再次提示。
                self.remote.lost = false;
                self.notify(t!(
                    "workspace-remote-failed",
                    host = target.as_str(),
                    error = error
                ));
            }
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

    /// 取消正在进行的连接（提示上的「取消」）：结束 `ssh`，后台的连接随即以失败返回。
    pub(in crate::app) fn cancel_remote(&mut self) {
        if let Some(abort) = &self.remote.abort {
            self.remote.declined = true;
            abort.abort();
        }
    }

    /// 命令「断开 SSH 连接」：关闭打开的远端目录，并断开到该主机的连接。
    ///
    /// 连接是进程内各窗口共用的：其他窗口打开着同一主机上的目录时，它们会提示连接已断开。
    pub(in crate::app) fn remote_disconnect(&mut self) {
        let target = (self.editor.root())
            .and_then(winc_backend::locate)
            .map(|location| location.target);
        let Some(target) = target else {
            return;
        };
        // 已提示过：关闭目录的过程中不再弹出连接中断的提示。
        self.remote.lost = true;
        self.close_folder();
        remote::disconnect(&target);
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
        // 主机之后全是数字的一段是端口，其后才是目录。
        assert_eq!(
            parsed("me@box:22022"),
            Some(("me@box:22022".to_owned(), ""))
        );
        assert_eq!(
            parsed("me@box:22022:"),
            Some(("me@box:22022".to_owned(), ""))
        );
        assert_eq!(
            parsed("me@box:22022:~/app"),
            Some(("me@box:22022".to_owned(), "~/app"))
        );
        assert_eq!(
            parsed("box:22:/srv:x"),
            Some(("box:22".to_owned(), "/srv:x"))
        );
        assert_eq!(parsed("box:~/2024"), Some(("box".to_owned(), "~/2024")));
        // 不是有效的端口：不当作目录去连接。
        assert_eq!(parsed("box:0"), None);
        assert_eq!(parsed("box:99999"), None);
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
    fn hosts_come_from_the_host_lines_of_the_ssh_configuration() {
        let config = "\
# comment
Host devbox
  HostName 10.0.0.5
  User me
host = staging  \"build box\" *.internal !bastion
Host *
  ServerAliveInterval 30
HOST=prod devbox
Match host other
Hostname not-a-host
Include extra
Host -oProxyCommand=x a;b
";
        // 通配和取反的规则、重复的名字、不能作为 `ssh` 主机参数的名字都不列出。
        assert_eq!(config_hosts(config), ["devbox", "staging", "prod"]);
        assert!(config_hosts("").is_empty());
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
        let home = open(&target, "~", None, None, &remote::Abort::default()).unwrap();
        assert!(winc_backend::fs::is_dir(&home), "{}", home.display());
        let sub = open(&target, ".ssh", None, None, &remote::Abort::default()).unwrap();
        assert_eq!(sub, home.join(".ssh"));
        let missing = open(
            &target,
            "/winc/no/such/folder",
            None,
            None,
            &remote::Abort::default(),
        )
        .unwrap_err();
        assert!(missing.contains("/winc/no/such/folder"), "{missing}");
        remote::disconnect(&target);
    }
}
