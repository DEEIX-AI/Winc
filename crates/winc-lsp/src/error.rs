//! 错误类型定义。

use std::fmt;

/// LSP 模块的错误类型。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// JSON-RPC 协议错误
    #[error("JSON-RPC error: {0}")]
    Rpc(#[from] crate::jsonrpc::Error),

    /// JSON 解析错误
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// IO 错误
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// 服务器未初始化
    #[error("Server not initialized")]
    NotInitialized,

    /// 服务器未找到
    #[error("Server not found: {0}")]
    ServerNotFound(String),

    /// 请求超时
    #[error("Request timed out: {0:?}")]
    Timeout(crate::jsonrpc::RequestId),

    /// 服务器进程已退出
    #[error("Server process exited")]
    ServerExited,

    /// 服务器关闭了连接
    #[error("Server closed the stream")]
    StreamClosed,

    /// 位置转换错误
    #[error("Position conversion error: {0}")]
    PositionConversion(String),

    /// 其他错误
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// LSP 模块的结果类型。
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// JSON-RPC 响应错误。
#[derive(Debug, Clone)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for ResponseError {}
