//! 外观：界面缩放、字体、字号和主题。

use super::*;

impl App {
    /// 返回界面缩放倍数（配置中的 `ui.zoom` 除以 100，未设置时为 1）。
    pub(crate) fn zoom(&self) -> f32 {
        f32::from(self.config.ui_zoom.unwrap_or(100)) / 100.0
    }

    /// 把界面缩放倍数交给渲染器和红绿灯；新窗口建好时与缩放改变时调用。
    pub(in crate::app) fn sync_zoom(&mut self) {
        let zoom = self.zoom();
        if let Some(renderer) = &mut self.frame.renderer {
            renderer.set_zoom(zoom);
        }
        #[cfg(target_os = "macos")]
        if let Some(lights) = &self.link.traffic_lights {
            lights.set_zoom(f64::from(zoom));
        }
        self.ime.last_area = None;
    }

    /// 按配置应用界面缩放：整个窗口按新的倍数绘制，窗口的物理尺寸不变。
    ///
    /// 布局按新的逻辑尺寸重新计算，红绿灯随顶栏的高度重新摆放；编辑区首行不变，滚动位置只限制
    /// 回有效范围。
    pub(in crate::app) fn apply_zoom(&mut self) {
        self.sync_zoom();
        self.place_traffic_lights();
        self.relayout();
        self.clamp_scroll();
        self.request_redraw();
    }

    /// 按当前字号重新测量字体度量；字号或字体变化后调用。
    pub(in crate::app) fn measure_fonts(&mut self) {
        self.metrics = TextMetrics::of(&self.frame.fonts, &self.fonts);
    }

    /// 按配置应用新的字号（逻辑像素）。
    ///
    /// 行高、可见行数和行号栏宽度随之变化；滚动位置限制回有效范围，并滚动到光标可见。终端的
    /// 行列数在下一次排版后随之变化。
    pub(in crate::app) fn apply_font_sizes(&mut self) {
        self.fonts = ViewFonts::of(&self.config);
        self.measure_fonts();

        self.clamp_scroll();
        self.ensure_caret_visible();
        self.request_redraw();
    }

    /// 应用主题选择：`Auto` 交由系统决定，`Dark`、`Light` 固定为指定外观。
    pub(in crate::app) fn apply_theme(&mut self, choice: ThemeChoice) {
        self.theme_choice = choice;
        // 系统菜单、提示框和面板按窗口外观显示深浅。
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        if let Some(window) = &self.window.handle {
            window.set_theme(forced_theme(&choice));
            #[cfg(target_os = "macos")]
            self.defer_traffic_lights();
        }

        let current = if self.palette.dark {
            Theme::Dark
        } else {
            Theme::Light
        };
        let theme = forced_theme(&choice).or_else(|| self.system_theme());
        self.set_theme(theme.unwrap_or(current));
    }

    /// 系统当前的外观，取不到时为深色；还没有窗口时返回 `None`。
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    pub(in crate::app) fn system_theme(&self) -> Option<Theme> {
        let window = self.window.handle.as_ref()?;
        Some(window.theme().unwrap_or(Theme::Dark))
    }

    /// 门户报告的系统外观（见 `crate::color_scheme`），取不到时为深色。
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(in crate::app) fn system_theme(&self) -> Option<Theme> {
        Some(crate::color_scheme::current().unwrap_or(Theme::Dark))
    }

    /// 系统的外观变成了 `theme`：没有固定主题且深浅不同时跟随。
    pub(crate) fn system_theme_changed(&mut self, theme: Theme) {
        #[cfg(target_os = "macos")]
        self.defer_traffic_lights();
        if forced_theme(&self.theme_choice).is_none() && self.palette.dark != (theme == Theme::Dark)
        {
            self.set_theme(theme);
        }
    }

    /// `AppKit` 可能在外观通知返回后重设标题栏高度，下一轮事件再摆放按钮。
    #[cfg(target_os = "macos")]
    fn defer_traffic_lights(&self) {
        if let Some(proxy) = &self.link.proxy {
            let _ = proxy.send_event(UserEvent::RepositionTrafficLights);
        }
    }

    /// 深色（`dark`）或浅色外观时用的主题在列表（[`crate::theme::themes`]）里的位置。
    pub(crate) fn theme_slot(&self, dark: bool) -> usize {
        let id = if dark {
            &self.config.dark_theme
        } else {
            &self.config.light_theme
        };
        crate::theme::theme_index(dark, id.as_deref())
    }

    /// 深色（`dark`）或浅色外观时用的主题。
    pub(crate) fn theme_palette(&self, dark: bool) -> &'static Palette {
        crate::theme::themes(dark)[self.theme_slot(dark)].palette
    }

    /// 切到 `theme` 对应的主题，终端跟着换配色。
    pub(in crate::app) fn set_theme(&mut self, theme: Theme) {
        self.palette = self.theme_palette(theme == Theme::Dark);
        // 系统标题栏的深浅。
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(window) = &self.window.handle {
            window.set_theme(Some(theme));
        }
        self.terminal_theme_changed();

        if let Some(renderer) = &mut self.frame.renderer {
            renderer.set_background(self.palette.frame_bg);
        }

        self.request_redraw();
    }

    /// 按配置重新加载字体并刷新度量。
    ///
    /// # Errors
    ///
    /// 渲染层无法使用配置的字体时，返回面向用户的错误说明。
    pub(in crate::app) fn apply_fonts(&mut self) -> Result<(), String> {
        let fonts = font_set(&self.config);
        let unusable = || winc_i18n::t!("workspace-font-unusable").to_owned();
        // 有渲染器时就地换字体，共用这份字体的窗口一同更新。
        match &mut self.frame.renderer {
            Some(renderer) => renderer.set_fonts(&fonts).map_err(|_| unusable())?,
            None => self.frame.fonts = FontSystem::new(&fonts).map_err(|_| unusable())?,
        }
        // 预览的排版和测量结果按旧字体算出，全部重排。
        self.preview.markdown.layout = None;
        self.preview.markdown.memo = Default::default();

        self.apply_font_sizes();
        Ok(())
    }
}

/// 返回配置中的编辑区字体、界面字体和终端字体的列表；未配置时为空。
///
/// 未配置时渲染层使用内置的系统字体路径表。按名字查询 CoreText 一次约 4–7 ms，
/// 路径表只需几次 `stat`，因此未配置时不调用 CoreText。终端字体与编辑区字体相同时同样为
/// 空，不重复加载。
pub(in crate::app) fn font_specs(config: &Config) -> [Vec<FontSpec>; 3] {
    let terminal = configured_fonts(&config.terminal_font);
    let editor = configured_fonts(&config.editor_font);
    let terminal = if terminal == editor {
        Vec::new()
    } else {
        terminal
    };
    [editor, configured_fonts(&config.ui_font), terminal]
}

/// 返回界面文字的字重（`ui.fontWeight`），未配置时为 400。
pub(in crate::app) fn ui_font_weight(config: &Config) -> u16 {
    config.ui_font_weight.unwrap_or(Config::FONT_WEIGHT)
}

/// 返回编辑区等宽文字的字重（`editor.fontWeight`），未配置时为 400。
pub(in crate::app) fn editor_font_weight(config: &Config) -> u16 {
    config.editor_font_weight.unwrap_or(Config::FONT_WEIGHT)
}

/// 返回终端文字的字重（`terminal.fontWeight`）；未设置时同编辑区字重，因为终端默认使用
/// 编辑区的字体。
pub(in crate::app) fn terminal_font_weight(config: &Config) -> u16 {
    config
        .terminal_font_weight
        .unwrap_or_else(|| editor_font_weight(config))
}

/// 返回界面与编辑区文字的字重；终端的字重在终端的字体组中（见 [`font_set`]）。
pub(in crate::app) fn font_weights(config: &Config) -> cpui_render::FontWeights {
    let mut weights = cpui_render::FontWeights::default();
    weights.ui = FontWeight(ui_font_weight(config));
    weights.mono = FontWeight(editor_font_weight(config));
    weights
}

/// 返回渲染层要加载的全部字体：界面与编辑区的字体和字重、汉字字形的语言，以及终端与
/// Markdown 正文的字体组（[`TERMINAL_FONT`]、[`PROSE_FONT`]）。
pub(crate) fn font_set(config: &Config) -> FontSet {
    let [mut mono, ui, terminal] = font_specs(config);
    // `WINC_FONT` 指定的字体文件排在编辑区字体的最前面，便于临时试用其他字体。
    if let Some(path) = std::env::var_os("WINC_FONT") {
        mono.insert(0, FontSpec::Path(path.into()));
    }
    mono.extend(preferred_mono());
    let mut fonts = FontSet::default();
    (fonts.mono, fonts.ui) = (mono, ui);
    fonts.weights = font_weights(config);
    // 测试不按界面语言选字体：Linux 上带语言查询的等宽字体可能是所装的中文字体，度量随机器
    // 变化。测试的界面语言默认是中文，不设语言时各平台都用拉丁等宽字体，结果可以复现。
    fonts.cjk_language = (!cfg!(test))
        .then(crate::cjk_language)
        .flatten()
        .map(str::to_owned);
    // 终端未单独配置字体时字体组为空，使用编辑区的字体。
    let terminal = FontStack::monospaced(terminal).weight(FontWeight(terminal_font_weight(config)));
    let terminal = fonts.add(terminal);
    let prose = fonts.add(FontStack::proportional(configured_fonts(
        &config.markdown_font,
    )));
    debug_assert_eq!((terminal, prose), (TERMINAL_FONT, PROSE_FONT));
    fonts
}

/// 返回排在编辑区字体之后、系统等宽字体之前的首选等宽字体，在 macOS 的用户与系统字体目录中
/// 按文件名查找。
///
/// 用固定的文件路径，不存在的文件由渲染层跳过；按名字查找在没有安装时要遍历全部字体。
fn preferred_mono() -> Vec<FontSpec> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let home = std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Fonts"));
    let dirs = home.into_iter().chain([PathBuf::from("/Library/Fonts")]);
    dirs.flat_map(|dir| {
        ["JetBrainsMono-Regular.ttf", "JetBrainsMono[wght].ttf"]
            .map(|file| FontSpec::Path(dir.join(file)))
    })
    .collect()
}

/// 把配置中以逗号分隔的字体名或字体文件路径转成 `FontSpec` 的列表，按书写顺序。
///
/// 未配置或只有空白时返回空列表，使用渲染层的默认字体。
pub(in crate::app) fn configured_fonts(configured: &Option<String>) -> Vec<FontSpec> {
    let requests = resolve_font_list(configured.as_deref());
    let spec = |request| match request {
        winc_config::FontRequest::Name(name) => FontSpec::Name(name),
        winc_config::FontRequest::Path(path) => FontSpec::Path(path),
    };
    requests.into_iter().map(spec).collect()
}

/// 配置的字体列表中第一个不可用的字体，及其写法和原因；都可用或未配置时返回 `None`。
pub(in crate::app) fn unusable_font(
    configured: &Option<String>,
) -> Option<(FontSpec, String, FontError)> {
    configured_fonts(configured).into_iter().find_map(|spec| {
        let error = spec.check().err()?;
        let name = match &spec {
            FontSpec::Name(name) => name.clone(),
            FontSpec::Path(path) => path.display().to_string(),
            // 配置里的字体只有名字和路径两种写法。
            _ => return None,
        };
        Some((spec, name, error))
    })
}

/// 返回启动时固定给窗口的主题。
///
/// 配置中的设置优先，其次读取环境变量 `WINC_THEME=dark|light`；都没有时返回 `None`，由系统
/// 决定（界面跟随系统外观，见 [`App::system_theme_changed`]）。
pub(in crate::app) fn forced_theme(choice: &ThemeChoice) -> Option<Theme> {
    match choice {
        ThemeChoice::Dark => Some(Theme::Dark),
        ThemeChoice::Light => Some(Theme::Light),
        ThemeChoice::Auto => match std::env::var("WINC_THEME").as_deref() {
            Ok("dark") => Some(Theme::Dark),
            Ok("light") => Some(Theme::Light),
            _ => None,
        },
    }
}
