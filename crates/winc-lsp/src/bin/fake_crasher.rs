//! 启动后立即异常退出的假服务器：只给 `tests/lifecycle.rs` 测「服务器崩溃后连接报告
//! 断开」用。

fn main() {
    std::process::exit(1);
}
