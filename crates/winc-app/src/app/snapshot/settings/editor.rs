//! 「编辑器」页设置的测试：字重与长行参考线。

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
