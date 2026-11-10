//! JSON-RPC 2.0 协议实现。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 请求 ID。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(u64),
    String(String),
}

impl From<u64> for RequestId {
    fn from(id: u64) -> Self {
        Self::Number(id)
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{}", n),
            Self::String(s) => write!(f, "{}", s),
        }
    }
}

/// JSON-RPC 消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// JSON-RPC 请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    pub params: Option<Value>,
}

impl Request {
    pub fn new(id: RequestId, method: String, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method,
            params,
        }
    }
}

/// JSON-RPC 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

/// JSON-RPC 响应错误。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 通知。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Notification {
    pub fn new(method: String, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        }
    }
}

/// JSON-RPC 错误码。
#[allow(dead_code)]
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const SERVER_ERROR_START: i32 = -32099;
    pub const SERVER_ERROR_END: i32 = -32000;

    // LSP specific error codes
    pub const SERVER_NOT_INITIALIZED: i32 = -32002;
    pub const UNKNOWN_ERROR_CODE: i32 = -32001;
    pub const REQUEST_CANCELLED: i32 = -32800;
    pub const CONTENT_MODIFIED: i32 = -32801;
}

/// JSON-RPC 错误类型。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Invalid request")]
    InvalidRequest,

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Invalid params")]
    InvalidParams,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Server error {code}: {message}")]
    Server { code: i32, message: String },
}

impl Error {
    pub fn to_response_error(self) -> ResponseError {
        match self {
            Self::Parse(msg) => ResponseError {
                code: error_codes::PARSE_ERROR,
                message: msg,
                data: None,
            },
            Self::InvalidRequest => ResponseError {
                code: error_codes::INVALID_REQUEST,
                message: "Invalid request".to_string(),
                data: None,
            },
            Self::MethodNotFound(method) => ResponseError {
                code: error_codes::METHOD_NOT_FOUND,
                message: format!("Method not found: {}", method),
                data: None,
            },
            Self::InvalidParams => ResponseError {
                code: error_codes::INVALID_PARAMS,
                message: "Invalid params".to_string(),
                data: None,
            },
            Self::Internal(msg) => ResponseError {
                code: error_codes::INTERNAL_ERROR,
                message: msg,
                data: None,
            },
            Self::Server { code, message } => ResponseError {
                code,
                message,
                data: None,
            },
        }
    }
}
