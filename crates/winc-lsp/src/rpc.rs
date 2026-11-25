//! JSON-RPC 分帧与报文信封：`Content-Length` 头的解析与消息编码，处理粘包与半包。
//!
//! 一帧的字节形态是「头 + 空行 + 体」：头是若干行 `键: 值`（`Content-Length` 必有，
//! `Content-Type` 等忽略），行与行之间、空行都是 `\r\n`。体是单行 JSON（ [`Json::compact`]
//! 写出的形态）。[`Decoder`] 是流式的：读线程把任意大小的字节块喂进来，每次取走零或一条
//! 完整报文，粘包自动拆开、半包留在缓冲区里等下一块。

use winc_json::Json;

/// JSON-RPC 报文信封：请求、响应或通知。
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// 请求：带 id，需要对方答复。客户端发给服务器的握手与补全是请求；服务器也会发请求
    /// （`workspace/configuration` 等），调用方应回 [`Message::Response`]。
    Request {
        /// 请求 id；响应时要原样带回。
        id: Id,
        /// 方法名。
        method: String,
        /// 参数；缺省为 `None`（不写字段）。
        params: Option<Json>,
    },
    /// 响应：对某条请求的答复。`result` 与 `error` 恰有一个（字段缺席还是 `None`）。
    Response {
        /// 对应请求的 id。
        id: Id,
        /// 成功结果；字段写了（哪怕是 `null`）就是 `Some`。
        result: Option<Json>,
        /// 失败信息。
        error: Option<RpcError>,
    },
    /// 通知：无 id，不需要答复。文档同步与诊断推送都是通知。
    Notification {
        /// 方法名。
        method: String,
        /// 参数。
        params: Option<Json>,
    },
}

/// JSON-RPC 的请求 id：数字或字符串。服务器用什么发来，响应就原样带回去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Id {
    /// 数字 id。
    Number(i64),
    /// 字符串 id。
    Text(String),
}

impl Id {
    /// 从 `Json` 解析 id；既不是数字也不是字符串时返回 `None`。
    fn from_json(json: &Json) -> Option<Id> {
        match json {
            Json::Number(n) => Some(Id::Number(*n as i64)),
            Json::String(text) => Some(Id::Text(text.clone())),
            _ => None,
        }
    }

    /// 写出 id。
    fn to_json(&self) -> Json {
        match self {
            Id::Number(n) => Json::Number(*n as f64),
            Id::Text(text) => Json::String(text.clone()),
        }
    }
}

/// JSON-RPC 错误响应的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// 错误码（如 -32601：方法不存在）。
    pub code: i64,
    /// 错误消息。
    pub message: String,
}

impl Message {
    /// 解析报文信封；不是对象、既没方法又没 id、响应里 result 与 error 都缺席时返回
    /// `None`（按「无法理解就丢弃」处理）。
    pub fn from_json(json: &Json) -> Option<Message> {
        let map = json.as_object()?;
        if let Some(method) = map.get("method").and_then(Json::as_str) {
            let method = method.to_owned();
            let params = map.get("params").cloned();
            return match map.get("id") {
                Some(id) => Some(Message::Request {
                    id: Id::from_json(id)?,
                    method,
                    params,
                }),
                None => Some(Message::Notification { method, params }),
            };
        }
        let id = Id::from_json(map.get("id")?)?;
        let error = map
            .get("error")
            .and_then(Json::as_object)
            .and_then(|error| {
                Some(RpcError {
                    code: error.get("code").and_then(Json::as_number)? as i64,
                    message: error.get("message").and_then(Json::as_str)?.to_owned(),
                })
            });
        // 字段写没写要看键在不在，不能用值判空（`result: null` 是合法的成功响应）。
        let result = map.get("result").cloned();
        if result.is_none() && error.is_none() {
            return None;
        }
        Some(Message::Response { id, result, error })
    }

    /// 写出报文信封。
    pub fn to_json(&self) -> Json {
        let mut fields: Vec<(&str, Json)> = vec![("jsonrpc", Json::String("2.0".into()))];
        match self {
            Message::Request { id, method, params } => {
                fields.push(("id", id.to_json()));
                fields.push(("method", Json::String(method.clone())));
                if let Some(params) = params {
                    fields.push(("params", params.clone()));
                }
            }
            Message::Response { id, result, error } => {
                fields.push(("id", id.to_json()));
                if let Some(result) = result {
                    fields.push(("result", result.clone()));
                }
                if let Some(error) = error {
                    fields.push((
                        "error",
                        Json::object([
                            ("code", Json::Number(error.code as f64)),
                            ("message", Json::String(error.message.clone())),
                        ]),
                    ));
                }
            }
            Message::Notification { method, params } => {
                fields.push(("method", Json::String(method.clone())));
                if let Some(params) = params {
                    fields.push(("params", params.clone()));
                }
            }
        }
        Json::object(fields)
    }

    /// 方法名；响应没有方法。
    pub fn method(&self) -> Option<&str> {
        match self {
            Message::Request { method, .. } | Message::Notification { method, .. } => Some(method),
            Message::Response { .. } => None,
        }
    }
}

/// 把一条报文编码成完整的帧字节（头 + 空行 + 单行体）。
pub fn frame(message: &Message) -> Vec<u8> {
    let body = message.to_json().compact();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// 流式分帧器。读线程把收到的字节喂给 [`Decoder::push`]，再反复调用
/// [`Decoder::next_message`] 取走完整报文。
#[derive(Debug, Default)]
pub struct Decoder {
    /// 还没解析完的字节。
    buffer: Vec<u8>,
}

impl Decoder {
    /// 新建空的分帧器。
    pub fn new() -> Decoder {
        Decoder { buffer: Vec::new() }
    }

    /// 喂入一块字节；块的大小与帧边界无关（粘包、半包都行）。
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// 取出下一条完整报文；没有完整帧时返回 `None`。
    ///
    /// 坏头（解析不出 `Content-Length`）或帧体不是 UTF-8/合法报文时跳过该帧继续取下一条，
    /// 连接不断：这只会在字节流失步时发生，管道本身不会产生坏帧。
    pub fn next_message(&mut self) -> Option<Message> {
        loop {
            let header_end = self.buffer.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
            let Some(length) = content_length(&self.buffer[..header_end]) else {
                // 坏头：丢弃这个头块，从下一处空行重新同步。
                self.buffer.drain(..header_end);
                continue;
            };
            let body_end = header_end + length;
            if self.buffer.len() < body_end {
                return None; // 半包：等下一块字节。
            }
            let body = self.buffer[header_end..body_end].to_vec();
            self.buffer.drain(..body_end);
            // 体不是 UTF-8 或不是合法报文的帧整个跳过。
            if let Some(message) = String::from_utf8(body)
                .ok()
                .and_then(|body| Json::parse(&body))
                .and_then(|json| Message::from_json(&json))
            {
                return Some(message);
            }
        }
    }
}

/// 从头块字节里读 `Content-Length`；头名大小写不敏感，其他头（如 `Content-Type`）忽略。
fn content_length(header: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(header).ok()?;
    for line in text.split("\r\n") {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests;
