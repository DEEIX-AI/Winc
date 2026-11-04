//! [`Client`] 的测试：覆盖辅助进程中途退出后，连接报断开、发消息不阻塞。

use std::time::{Duration, Instant};

use super::*;

/// 辅助进程退出后，`poll` 在 5 秒内返回 `Closed` 并调用过 `wake`，断开后发送消息不阻塞。
#[test]
fn a_helper_that_goes_away_reads_as_closed() {
    let woke = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&woke);
    // 一启动就退出的程序。
    #[cfg(windows)]
    let program = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join(r"System32\hostname.exe");
    #[cfg(not(windows))]
    let program = std::path::PathBuf::from("/usr/bin/true");
    let client = Client::start(&program, move || {
        flag.store(true, Ordering::Release);
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "等不到连接断开");
        if woke.load(Ordering::Acquire) && matches!(client.poll(), Poll::Closed) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    // 断开以后发送消息不返回错误，也不阻塞线程。
    client.send(&ToHelper::Credit { tab: 1 });
    drop(client);
}
