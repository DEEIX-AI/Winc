//! 「编辑器」页设置的测试：字重。

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
