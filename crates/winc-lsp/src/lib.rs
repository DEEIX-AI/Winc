//! LSP 客户端实现：管理语言服务器进程，处理 JSON-RPC 协议通信。
//!
//! 核心类型：
//! - [`Registry`]：管理多个 LSP 服务器实例
//! - [`Client`]：单个语言服务器的连接和生命周期
//! - [`Transport`]：处理 stdio 协议的读写
//!
//! 采用线程池模型而非 tokio，与 Winc 现有架构保持一致。

mod client;
mod error;
mod jsonrpc;
mod registry;
mod sync;
mod transport;
mod types;

pub use client::{Client, ClientId};
pub use error::{Error, Result};
pub use jsonrpc::{Notification, Request, RequestId, Response};
pub use lsp_types as lsp;
pub use registry::Registry;
pub use sync::{full_document_sync, transaction_to_lsp_changes};
pub use transport::ServerMessage;
pub use types::{lsp_pos_to_pos, lsp_range_to_range, pos_to_lsp_pos, range_to_lsp_range};

/// LSP 协议的位置编码方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OffsetEncoding {
    /// UTF-8 字节偏移
    Utf8,
    /// UTF-32 字符偏移
    Utf32,
    /// UTF-16 代码单元偏移（默认）
    #[default]
    Utf16,
}

impl OffsetEncoding {
    /// 从 LSP 协议的位置编码名称解析。
    pub fn from_lsp(encoding: &str) -> Option<Self> {
        match encoding {
            "utf-8" => Some(Self::Utf8),
            "utf-16" => Some(Self::Utf16),
            "utf-32" => Some(Self::Utf32),
            _ => None,
        }
    }

    /// 转换为 LSP 协议的位置编码名称。
    pub fn to_lsp(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16 => "utf-16",
            Self::Utf32 => "utf-32",
        }
    }
}
