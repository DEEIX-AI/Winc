//! 「编辑器」页设置的测试：字重、长行参考线与换行符。

use super::*;

/// 编辑区的字重在「编辑器」页中单独设置，默认 400：与界面字重互不影响，回到 400 时从配置中
/// 去掉该项。
#[test]
fn the_editor_font_weight_is_set_apart_from_the_interface() {
    let dir = sample_folder("settings-editor-font-weight");
    let mut app = app_with(&dir.join("short.rs"), false);
    assert_eq!(Setting::EditorFontWeight.page(), SettingsPage::Editor);
    app.config.ui_font_weight = Some(600);
    assert_eq!(app.setting_value(Setting::EditorFontWeight).0, "400");
    assert_eq!(app.font_weights().editor, 400);

    app.settings_adjust(Setting::EditorFontWeight, true);
    assert_eq!(app.config.editor_font_weight, Some(500));
    assert_eq!(app.config.ui_font_weight, Some(600));
    let weights = app.font_weights();
    assert_eq!((weights.ui, weights.editor), (600, 500));
    app.settings_adjust(Setting::FontWeight, true);
    assert_eq!(app.setting_value(Setting::EditorFontWeight).0, "500");
    app.settings_adjust(Setting::EditorFontWeight, false);
    assert_eq!(app.config.editor_font_weight, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 长行参考线：设置中以逗号分隔的列号按从小到大保存，清空后不显示；无法识别时留在输入框中并
/// 说明原因，配置不变。参考线画在第 N 列之后。
#[test]
fn rulers_are_typed_as_columns_and_drawn_after_them() {
    let dir = sample_folder("settings-editor-rulers");
    let mut app = app_with(&dir.join("short.rs"), false);
    assert_eq!(Setting::EditorRulers.page(), SettingsPage::Editor);
    let commit = |app: &mut App, text: &str| {
        app.settings_start_editing(Setting::EditorRulers);
        app.edit_field(|f| {
            f.set_text(text);
            true
        });
        app.settings_commit_field();
    };
    commit(&mut app, "120，80 80、100");
    assert_eq!(app.config.editor_rulers, [80, 100, 120]);
    app.settings_start_editing(Setting::EditorRulers);
    let shown = app
        .settings
        .editing
        .as_ref()
        .map(|(_, f)| f.text().to_string());
    assert_eq!(shown.as_deref(), Some("80, 100, 120"));
    app.settings_commit_field();

    commit(&mut app, "80, abc");
    assert!(app.settings.error.is_some());
    assert_eq!(app.config.editor_rulers, [80, 100, 120]);
    commit(&mut app, "0");
    assert!(app.settings.error.is_some());
    commit(&mut app, "");
    assert!(app.config.editor_rulers.is_empty() && app.settings.error.is_none());

    // 画在第 4 列之后：该处的像素是参考线的颜色，旁边仍是底色。
    app.close_settings();
    app.config.editor_rulers = vec![4];
    paint(&mut app);
    let text = app.layout.text;
    let x = (text.x + crate::theme::TEXT_PAD + 4.0 * app.metrics.cell).round();
    let y = text.bottom() - 4.0;
    assert_ne!(pixel(&app, (x + 0.5, y)), pixel(&app, (x + 3.5, y)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 换行符：打开设置后各行行尾（第一个字符之后的一格）画出该行的换行符，CRLF 与 LF 的标记
/// 不同；最后一行没有换行符，不画。
#[test]
fn line_endings_are_marked_after_each_line() {
    let dir = sample_folder("settings-line-endings");
    let path = dir.join("mixed.txt");
    std::fs::write(&path, "a\r\nb\nc\rd").unwrap();
    let mut app = app_with(&path, false);
    // 第 `line` 行行尾那一格中与同一行右侧空白处不同的像素数（光标所在行另有底色）。
    let ink = |app: &mut App, line: usize| -> usize {
        paint(app);
        let (text, cell, lh) = (app.layout.text, app.metrics.cell, app.fonts.line_height);
        let x0 = text.x + crate::theme::TEXT_PAD + cell;
        let y0 = text.y + line as f32 * lh;
        let blank = pixel(app, (x0 + 4.0 * cell + 0.5, y0 + lh / 2.0));
        let (columns, rows) = (cell.ceil() as usize, lh.floor() as usize);
        let points = (0..columns).flat_map(|dx| (0..rows).map(move |dy| (dx, dy)));
        let at = |(dx, dy): (usize, usize)| (x0 + dx as f32 + 0.5, y0 + dy as f32 + 0.5);
        points.filter(|&p| pixel(app, at(p)) != blank).count()
    };
    let off: Vec<usize> = (0..4).map(|line| ink(&mut app, line)).collect();
    assert!(off.iter().all(|&n| n == 0), "{off:?}");

    app.settings_toggle(Setting::RenderLineEndings);
    assert_eq!(app.config.render_line_endings, Some(true));
    let marks: Vec<usize> = (0..4).map(|line| ink(&mut app, line)).collect();
    assert!(marks[..3].iter().all(|&n| n > 0), "{marks:?}");
    assert_ne!(marks[0], marks[1], "CRLF 与 LF 的标记不同");
    assert_eq!(marks[3], 0);
    let _ = std::fs::remove_dir_all(&dir);
}
