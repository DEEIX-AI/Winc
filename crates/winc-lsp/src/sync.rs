//! 文档同步：将 Winc 的编辑操作转换为 LSP 协议消息。

use crate::{types, OffsetEncoding, Result};
use lsp_types::TextDocumentContentChangeEvent;
use ropey::Rope;
use winc_core::{Change, Transaction};

/// 将 Transaction 转换为 LSP 文档变更事件。
///
/// # 参数
/// - `transaction`: Winc 的事务对象
/// - `rope`: 应用事务**之前**的文档内容
/// - `encoding`: LSP 位置编码方式
/// - `incremental`: 是否生成增量更新（false = 发送全文）
pub fn transaction_to_lsp_changes(
    transaction: &Transaction,
    rope: &Rope,
    encoding: OffsetEncoding,
    incremental: bool,
) -> Result<Vec<TextDocumentContentChangeEvent>> {
    if !incremental {
        // 全文同步：直接发送完整文档
        return Ok(vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: rope.to_string(),
        }]);
    }

    // 增量同步：将每个 Change 转换为 LSP 变更
    changes_to_lsp_changes(&transaction.changes, rope, encoding)
}

/// 将 Change 列表转换为 LSP 文档变更事件。
fn changes_to_lsp_changes(
    changes: &[Change],
    rope: &Rope,
    encoding: OffsetEncoding,
) -> Result<Vec<TextDocumentContentChangeEvent>> {
    let mut lsp_changes = Vec::new();

    for change in changes {
        let start = change.start;
        let end = start + change.removed.chars().count();

        // 转换为 LSP Range
        let range = types::range_to_lsp_range(rope, start, end, encoding)?;

        lsp_changes.push(TextDocumentContentChangeEvent {
            range: Some(range),
            range_length: Some(change.removed.len() as u32),
            text: change.inserted.clone(),
        });
    }

    Ok(lsp_changes)
}

/// 为 LSP 同步准备文档内容的完整快照。
pub fn full_document_sync(rope: &Rope) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: rope.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winc_core::Selection;

    #[test]
    fn test_full_sync() {
        let rope = Rope::from_str("Hello, World!");
        let change = full_document_sync(&rope);

        assert_eq!(change.text, "Hello, World!");
        assert!(change.range.is_none());
    }
}
