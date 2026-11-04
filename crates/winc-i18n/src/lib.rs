//! 界面文案的多语言支持：界面语言的选择、消息的查找和格式化。
//!
//! 文案以 Fluent 格式写在 `locales/<语言标签>/<领域>.ftl`，编译时嵌入程序。代码以消息 id 引用文案：
//! `t!("id")` 返回 `&'static str`，`t!("id", name = value)` 代入参数后返回 `String`。

mod catalog;
mod locale;
#[cfg(test)]
mod tests;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;

#[doc(hidden)]
pub use fluent_bundle::FluentArgs;
pub use locale::{Locale, LocaleChoice};

/// 一种语言的全部消息。
type Bundle = FluentBundle<FluentResource>;

/// 当前界面语言在 [`Locale::ALL`] 中的序号。
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// 设定界面语言，之后的查找都使用这种语言。
pub fn set_locale(locale: Locale) {
    CURRENT.store(locale.index() as u8, Ordering::Relaxed);
}

/// 返回当前的界面语言。
pub fn locale() -> Locale {
    Locale::ALL[usize::from(CURRENT.load(Ordering::Relaxed))]
}

/// 返回 `locale` 的消息集合；首次使用时解析该语言的全部 `.ftl` 文件。
///
/// # Panics
///
/// 语言包有语法错误或重复的消息 id 时 panic；测试覆盖每个语言包，发布的程序中不会出现。
fn bundle(locale: Locale) -> &'static Bundle {
    static BUNDLES: [OnceLock<Bundle>; Locale::COUNT] = [const { OnceLock::new() }; Locale::COUNT];
    BUNDLES[locale.index()].get_or_init(|| {
        let mut bundle = Bundle::new_concurrent(vec![locale.langid()]);
        // 不在代入的参数两侧插入 Unicode 方向隔离符（U+2068、U+2069）：两种语言都是从左到右书写，
        // 隔离符会作为不可见字符进入复制的文本和宽度计算。
        bundle.set_use_isolating(false);
        for (file, source) in catalog::sources(locale) {
            let resource = FluentResource::try_new((*source).to_owned())
                .unwrap_or_else(|(_, errors)| panic!("{}/{file}.ftl：{errors:?}", locale.tag()));
            bundle
                .add_resource(resource)
                .unwrap_or_else(|errors| panic!("{}/{file}.ftl：{errors:?}", locale.tag()));
        }
        bundle
    })
}

/// 所有消息都能用的变量 `$platform`：当前系统的名称，取 `macos`、`windows` 或 `linux`。
///
/// 其他类 Unix 系统也记为 `linux`。用于各系统叫法不同的词，例如 `common-trash`（「废纸篓」与
/// 「回收站」）。
pub const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

/// 在 `locale` 中查找消息 `id` 并代入 `args`，总是额外带上 [`PLATFORM`]。
///
/// 当前语言缺少该消息时改用源语言 [`Locale::SOURCE`]；两者都没有时返回 `id` 本身，
/// 使界面上出现可辨认的 id 而不是空白。
fn lookup(locale: Locale, id: &str, args: Option<&FluentArgs>) -> String {
    let mut all = FluentArgs::new();
    all.set("platform", PLATFORM);
    for (name, value) in args.into_iter().flat_map(FluentArgs::iter) {
        all.set(name.to_owned(), value.clone());
    }
    for locale in [locale, Locale::SOURCE] {
        let bundle = bundle(locale);
        if let Some(pattern) = bundle.get_message(id).and_then(|message| message.value()) {
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, Some(&all), &mut errors);
            debug_assert!(errors.is_empty(), "{}：{id}：{errors:?}", locale.tag());
            return text.into_owned();
        }
    }
    debug_assert!(false, "语言包中没有消息 {id}");
    id.to_owned()
}

/// 在当前语言中查找消息 `id` 并代入 `args`，供 [`t!`] 的带参数形式调用。
#[doc(hidden)]
pub fn format(id: &str, args: &FluentArgs) -> String {
    lookup(locale(), id, Some(args))
}

/// 不带参数的消息在一处调用点上的查找结果，每种语言各占一个槽位，供 [`t!`] 调用。
///
/// 结果在首次查找时生成并在程序生命周期内保留。调用点数量和语言数量都是固定的，
/// 占用内存有上限；之后的查找不加锁、不分配内存，可在每帧的绘制中调用。
#[doc(hidden)]
pub struct Slot([OnceLock<&'static str>; Locale::COUNT]);

impl Slot {
    /// 返回尚未查找过的槽位。
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Slot([const { OnceLock::new() }; Locale::COUNT])
    }

    /// 返回消息 `id` 在当前语言中的文本。
    pub fn get(&self, id: &'static str) -> &'static str {
        let locale = locale();
        self.0[locale.index()].get_or_init(|| lookup(locale, id, None).leak())
    }
}

/// 按消息 id 取得当前语言的界面文案。
///
/// - `t!("id")`：不带参数的消息，返回 `&'static str`。
/// - `t!("id", name = value, …)`：代入参数后返回 `String`。参数值可以是字符串或数字，
///   数字参数参与 Fluent 的复数选择（英文的单复数等）。
#[macro_export]
macro_rules! t {
    ($id:literal) => {{
        static SLOT: $crate::Slot = $crate::Slot::new();
        SLOT.get($id)
    }};
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::FluentArgs::new();
        $(args.set(stringify!($name), $value);)+
        $crate::format($id, &args)
    }};
}
