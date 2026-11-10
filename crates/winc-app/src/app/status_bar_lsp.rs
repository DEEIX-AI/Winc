//! 状态栏 LSP 信息显示。

use crate::app::App;

impl App {
    /// 在状态栏显示 LSP 诊断统计。
    pub(crate) fn render_lsp_status(&self) -> Option<String> {
        if !self.editor.is_lsp_enabled() {
            return None;
        }

        let (errors, warnings, infos, hints) = self.diagnostic_stats();

        if errors + warnings + infos + hints == 0 {
            return Some("✓ No issues".to_string());
        }

        let mut parts = Vec::new();

        if errors > 0 {
            parts.push(format!("❌ {}", errors));
        }
        if warnings > 0 {
            parts.push(format!("⚠️ {}", warnings));
        }
        if infos > 0 {
            parts.push(format!("ℹ️ {}", infos));
        }
        if hints > 0 {
            parts.push(format!("💡 {}", hints));
        }

        Some(parts.join("  "))
    }

    /// 检查 LSP 服务器是否正在运行。
    pub(crate) fn lsp_server_status(&self) -> Option<String> {
        let doc = self.editor.active()?;
        let lsp_state = doc.lsp_state.as_ref()?;
        let registry = self.editor.lsp_registry.as_ref()?;
        let client = registry.get(lsp_state.client_id)?;

        if client.is_initialized() {
            Some(format!("LSP: {} ✓", client.name()))
        } else {
            Some(format!("LSP: {} initializing...", client.name()))
        }
    }
}
