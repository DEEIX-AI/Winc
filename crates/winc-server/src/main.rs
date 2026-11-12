//! 运行在远端主机上的服务进程。
//!
//! 由本机的 winc 经 `ssh` 启动，从标准输入读取请求，把应答写到标准输出（见
//! `winc_backend::remote::serve`）。不监听端口，不写任何文件；标准输入结束（连接断开）时
//! 退出。目前只支持 Unix 主机。
//!
//! 以 `watch <目录>` 为参数时改为监视该目录，把变化写到标准输出（见 `winc_watch`）：由服务
//! 进程在需要监视时再次运行自身，连接断开或不再需要时结束它。

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|arg| arg == "watch") {
        let Some(root) = args.next().map(std::path::PathBuf::from) else {
            return ExitCode::FAILURE;
        };
        return match winc_watch::run(&root, false) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("winc-server: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(unix)]
    {
        // 输出由多个线程共用，不能持有标准输出的锁；每一帧写完都会刷新。
        let input = std::io::BufReader::new(std::io::stdin().lock());
        let output = std::io::BufWriter::new(std::io::stdout());
        match winc_backend::remote::serve(input, output) {
            Ok(()) => ExitCode::SUCCESS,
            // 不合协议的请求或输入输出错误：没有可以应答的对象，直接结束。
            Err(_) => ExitCode::FAILURE,
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("winc-server: this platform is not supported");
        ExitCode::FAILURE
    }
}
