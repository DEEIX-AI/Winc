//! Windows 的端到端测试：启动真正的 `winc-pty` 在 ConPTY 中运行 `cmd.exe`，按帧还原屏幕。

#![allow(unsafe_code)] // 查询是否运行在 Wine 中
#![cfg(windows)]

mod common;

use std::time::{Duration, Instant};

use common::{Client, WAIT};
use winc_pty::protocol::ToHelper;

/// 是否运行在 Wine 中。
///
/// Wine 的伪终端不转发程序的输出，这些测试需要在真实的 Windows 上运行。
fn under_wine() -> bool {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    // SAFETY: 两个参数都是以 NUL 结尾的字符串常量，在整个调用期间有效；只查询模块句柄和
    // 函数地址，不调用找到的函数。
    unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
        !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
    }
}

/// 运行在 Wine 中时输出提示并返回 `true`。
fn skip_under_wine() -> bool {
    let wine = under_wine();
    if wine {
        eprintln!("跳过：Wine 的伪终端不转发程序的输出，需要在 Windows 上运行");
    }
    wine
}

/// 系统 `cmd.exe` 的路径。
fn cmd() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into())
}

#[test]
fn a_program_runs_in_a_pseudo_console_and_reports_its_exit_code() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    c.spawn(1, &cmd(), &["/c", "echo hello from conpty& exit 3"]);
    assert_eq!(c.until_exit_showing(|t| t.contains("hello from conpty")), 3);
}

#[test]
fn input_reaches_the_shell() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    c.spawn(1, &cmd(), &["/q", "/k"]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"echo typed %COMSPEC%\r".to_vec(),
        rest: 0,
    });
    c.until_text("typed ");
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"exit 5\r".to_vec(),
        rest: 0,
    });
    assert_eq!(c.until_exit(), 5);
}

/// 关闭标签：伪终端随之关闭，其中的程序以及它启动的程序一同结束，辅助进程随后退出。
#[test]
fn closing_a_tab_ends_everything_in_it() {
    if skip_under_wine() {
        return;
    }
    let mut c = Client::start();
    // `cmd` 中再启动一个等待输入的 `cmd`：关闭标签时两层都要结束。
    c.spawn(1, &cmd(), &["/q", "/k", "cmd /q /k"]);
    c.send(ToHelper::Input {
        tab: 1,
        bytes: b"echo ready\r".to_vec(),
        rest: 0,
    });
    c.until_text("ready");
    c.send(ToHelper::Close { tab: 1 });
    let deadline = Instant::now() + WAIT + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = c.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "辅助进程没有退出");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
}
