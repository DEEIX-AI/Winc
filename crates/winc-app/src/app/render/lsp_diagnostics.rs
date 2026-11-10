//! LSP 诊断信息的渲染：波浪线、gutter 图标。

use crate::app::render::Render;
use crate::app::App;
use winc_editor::{DiagnosticSeverity, Document};
use winc_render::canvas::Canvas;
use winc_render::color::Color;

/// 诊断严重程度对应的颜色。
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticColors {
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub hint: Color,
}

impl Default for DiagnosticColors {
    fn default() -> Self {
        Self {
            error: Color::rgb(0xFF, 0x00, 0x00),      // 红色
            warning: Color::rgb(0xFF, 0xA5, 0x00),    // 橙色
            info: Color::rgb(0x00, 0x7A, 0xCC),       // 蓝色
            hint: Color::rgb(0x6A, 0x9F, 0xB5),       // 浅蓝灰
        }
    }
}

impl DiagnosticColors {
    /// 根据严重程度获取颜色。
    pub fn color_for(&self, severity: DiagnosticSeverity) -> Color {
        match severity {
            DiagnosticSeverity::Error => self.error,
            DiagnosticSeverity::Warning => self.warning,
            DiagnosticSeverity::Info => self.info,
            DiagnosticSeverity::Hint => self.hint,
        }
    }
}

impl Render {
    /// 渲染 LSP 诊断信息（在文本渲染之后调用）。
    pub(crate) fn draw_lsp_diagnostics(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        viewport_start: usize,
        viewport_end: usize,
        line_height: f32,
        scroll_y: f32,
    ) {
        let Some(lsp_state) = &doc.lsp_state else {
            return;
        };

        if lsp_state.diagnostics.is_empty() {
            return;
        }

        let colors = DiagnosticColors::default();
        let text = doc.content.text();

        for diag in &lsp_state.diagnostics {
            let (start, end) = diag.range;

            // 只渲染可见区域的诊断
            if end < viewport_start || start > viewport_end {
                continue;
            }

            let color = colors.color_for(diag.severity);

            // 获取诊断范围的行
            let start_line = text.char_to_line(start);
            let end_line = text.char_to_line(end);

            // 为每一行绘制波浪线
            for line_idx in start_line..=end_line {
                let line_start_char = text.line_to_char(line_idx);
                let line_end_char = if line_idx + 1 < text.len_lines() {
                    text.line_to_char(line_idx + 1)
                } else {
                    text.len_chars()
                };

                // 计算该行中诊断的起止位置
                let diag_start_in_line = start.max(line_start_char) - line_start_char;
                let diag_end_in_line = end.min(line_end_char) - line_start_char;

                if diag_start_in_line >= diag_end_in_line {
                    continue;
                }

                // 计算 Y 坐标
                let y = (line_idx as f32 * line_height) - scroll_y + line_height - 2.0;

                // 计算 X 坐标（简化版：假设等宽字体）
                let char_width = 8.0; // TODO: 从 font metrics 获取
                let x_start = 60.0 + (diag_start_in_line as f32 * char_width);
                let x_end = 60.0 + (diag_end_in_line as f32 * char_width);

                // 绘制波浪线
                self.draw_wavy_line(canvas, x_start, y, x_end, color);
            }
        }
    }

    /// 绘制波浪形下划线。
    fn draw_wavy_line(&self, canvas: &mut Canvas, x_start: f32, y: f32, x_end: f32, color: Color) {
        let wave_width = 4.0;
        let wave_height = 2.0;
        let mut x = x_start;

        canvas.set_color(color);

        while x < x_end {
            let x1 = x;
            let x2 = (x + wave_width * 0.5).min(x_end);
            let x3 = (x + wave_width).min(x_end);

            // 简化的波浪形：用线段模拟
            canvas.line(x1, y, x2, y - wave_height);
            canvas.line(x2, y - wave_height, x3, y);

            x += wave_width;
        }
    }

    /// 在 gutter 中绘制诊断图标。
    pub(crate) fn draw_diagnostic_gutter_icons(
        &mut self,
        canvas: &mut Canvas,
        doc: &Document,
        viewport_start_line: usize,
        viewport_end_line: usize,
        line_height: f32,
        scroll_y: f32,
        gutter_width: f32,
    ) {
        let Some(lsp_state) = &doc.lsp_state else {
            return;
        };

        if lsp_state.diagnostics.is_empty() {
            return;
        }

        let colors = DiagnosticColors::default();
        let text = doc.content.text();

        // 收集每一行的最严重诊断
        let mut line_diagnostics: std::collections::HashMap<usize, DiagnosticSeverity> =
            std::collections::HashMap::new();

        for diag in &lsp_state.diagnostics {
            let line = text.char_to_line(diag.range.0);

            if line < viewport_start_line || line > viewport_end_line {
                continue;
            }

            // 保留最严重的诊断
            line_diagnostics
                .entry(line)
                .and_modify(|existing| {
                    if Self::severity_priority(diag.severity) > Self::severity_priority(*existing) {
                        *existing = diag.severity;
                    }
                })
                .or_insert(diag.severity);
        }

        // 绘制图标
        for (line, severity) in line_diagnostics {
            let y = (line as f32 * line_height) - scroll_y + (line_height - 12.0) / 2.0;
            let x = gutter_width - 20.0;

            let color = colors.color_for(severity);
            self.draw_diagnostic_icon(canvas, x, y, severity, color);
        }
    }

    /// 返回严重程度的优先级（数字越大越严重）。
    fn severity_priority(severity: DiagnosticSeverity) -> u8 {
        match severity {
            DiagnosticSeverity::Error => 3,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Info => 1,
            DiagnosticSeverity::Hint => 0,
        }
    }

    /// 绘制诊断图标。
    fn draw_diagnostic_icon(
        &self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        severity: DiagnosticSeverity,
        color: Color,
    ) {
        let size = 12.0;
        canvas.set_color(color);

        match severity {
            DiagnosticSeverity::Error => {
                // 绘制 X 号
                canvas.line(x, y, x + size, y + size);
                canvas.line(x + size, y, x, y + size);
            }
            DiagnosticSeverity::Warning => {
                // 绘制三角形
                canvas.line(x + size / 2.0, y, x, y + size);
                canvas.line(x, y + size, x + size, y + size);
                canvas.line(x + size, y + size, x + size / 2.0, y);
            }
            DiagnosticSeverity::Info => {
                // 绘制圆圈
                canvas.circle(x + size / 2.0, y + size / 2.0, size / 2.0);
            }
            DiagnosticSeverity::Hint => {
                // 绘制点
                canvas.circle(x + size / 2.0, y + size / 2.0, size / 4.0);
            }
        }
    }
}

impl App {
    /// 获取光标位置的诊断信息（用于 tooltip）。
    pub fn diagnostic_at_cursor(&self) -> Option<String> {
        let doc = self.editor.active()?;
        let lsp_state = doc.lsp_state.as_ref()?;
        let cursor_pos = doc.selection.head();

        // 查找包含光标的诊断
        for diag in &lsp_state.diagnostics {
            if cursor_pos >= diag.range.0 && cursor_pos < diag.range.1 {
                let source = diag
                    .source
                    .as_ref()
                    .map(|s| format!("[{}] ", s))
                    .unwrap_or_default();
                return Some(format!("{}{}", source, diag.message));
            }
        }

        None
    }

    /// 获取所有诊断的统计信息。
    pub fn diagnostic_stats(&self) -> (usize, usize, usize, usize) {
        let doc = match self.editor.active() {
            Some(d) => d,
            None => return (0, 0, 0, 0),
        };

        let lsp_state = match &doc.lsp_state {
            Some(s) => s,
            None => return (0, 0, 0, 0),
        };

        let mut errors = 0;
        let mut warnings = 0;
        let mut infos = 0;
        let mut hints = 0;

        for diag in &lsp_state.diagnostics {
            match diag.severity {
                DiagnosticSeverity::Error => errors += 1,
                DiagnosticSeverity::Warning => warnings += 1,
                DiagnosticSeverity::Info => infos += 1,
                DiagnosticSeverity::Hint => hints += 1,
            }
        }

        (errors, warnings, infos, hints)
    }
}
