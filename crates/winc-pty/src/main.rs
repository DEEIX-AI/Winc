//! 终端的辅助进程：主程序打开终端面板时启动该进程，通过 stdin 和 stdout 上的二进制协议
//! （`winc_pty::protocol`）通信。
//!
//! 一个进程管理所有终端标签，关闭最后一个标签或者主程序退出（stdin 结束）时退出。

#[cfg(any(unix, windows))]
mod helper;

#[cfg(any(unix, windows))]
fn main() -> std::process::ExitCode {
    match helper::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("winc-pty: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn main() -> std::process::ExitCode {
    eprintln!("winc-pty: 此平台尚不支持内置终端");
    std::process::ExitCode::FAILURE
}
