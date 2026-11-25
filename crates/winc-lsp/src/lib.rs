//! LSP 客户端：语言服务器（首期 gopls，服务 Go）的协议、进程与位置换算。
//!
//! 分层上与 `winc-pty`、`winc-server` 同属「协议/进程型 crate」：只知道怎么跟一个子进程里的
//! 语言服务器说话，不知道编辑器里有什么。本 crate 不依赖 `winc-editor`（位置换算接收行表或
//! 偏移，不触碰文档模型），UI 与生命周期调度（何时启动、空闲退出、崩溃限频）都在 `winc-app`。
//!
//! 消息子集：只实现补全与诊断所需——`initialize`/`initialized`/`shutdown`/`exit`、
//! `textDocument/didOpen`/`didChange`/`didClose`、`textDocument/completion`、
//! `textDocument/publishDiagnostics`。序列化复用 `winc-json`（workspace 不引 serde），
//! 每个消息类型手写与 [`Json`](winc_json::Json) 的双向转换，转换正确性由
//! `tests/golden/` 里的真实 gopls 报文回放比对保证。
//!
//! 位置：编辑器全库用 char 偏移，LSP 用「行号 + UTF-16 code unit 列」，换算集中在
//! [`position`] 模块。进程与 IO 参照 `winc-pty` 的 Client：piped stdio、读写双线程、
//! 服务器消息经 wake 回调唤醒事件循环，由调用方在事件循环里 [`poll`](Connection::poll)。

pub mod connection;
pub mod message;
pub mod position;
pub mod rpc;
