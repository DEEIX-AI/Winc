//! 设置的测试辅助：按语言取格式化页上显示的行。

use super::*;

impl SettingsState {
    /// 返回语言 `id` 的格式化命令设置项；测试用（只有 Unix 上的格式化测试用到）。
    #[cfg(unix)]
    pub fn format_row(&self, id: &'static str) -> Option<&FormatRow> {
        self.tool_row(Setting::FormatCommand(id))
    }
}
