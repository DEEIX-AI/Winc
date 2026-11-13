//! 输入：键盘和输入法、鼠标、执行键位表里的命令、右键菜单、命令面板。
//!
//! 快捷键与命令面板的内容都来自键位表；鼠标事件按绘制时记下的可点击区域找到目标。

use super::*;

pub(super) mod actions;
pub(super) mod context_menu;
pub(super) mod keyboard;
pub(super) mod mouse;
pub(super) mod palette;
pub(super) mod touch;

/// 鼠标与修饰键的状态：位置、悬停与拖动的目标、按下时所在的控件。
pub(crate) struct Pointer {
    /// 鼠标悬停的目标。
    pub hover: Hover,
    /// 当前按下的修饰键。
    pub(crate) modifiers: ModifiersState,
    /// 左、右 ⌥ 键是否按下，用于按 `terminal.optionAsMeta` 判断视为 Meta 的一侧；
    /// winit 只在 macOS 上报告左右 ⌥ 的状态。
    pub(in crate::app) option_side: (bool, bool),
    /// 鼠标位置（逻辑像素）。
    pub(in crate::app) mouse: (f32, f32),
    /// 正在进行的鼠标拖动。
    pub(in crate::app) drag: Drag,
    /// 上一次鼠标按下，用于识别连击。
    pub(in crate::app) last_click: Option<Click>,
    /// 上一次在顶栏空白处的按下，用于识别双击。
    pub(in crate::app) title_click: Option<Click>,
    /// 鼠标按下时所在的标签关闭按钮（或中键按下的标签）；松开时仍在同一个标签上才关闭。
    pub(in crate::app) close_pressed: Option<DocumentId>,
    /// 鼠标按下时所在的窗口按钮；松开时仍在同一个按钮上才执行。
    ///
    /// 在按下时就关闭或最小化窗口，松开事件会落到下层窗口：下层窗口的标题栏按钮如果正好在
    /// 这个位置（这类按钮通常在松开时触发），就会被误触发。
    pub(in crate::app) control_pressed: Option<WindowControl>,
    /// 当前的鼠标指针形状。
    pub(in crate::app) cursor_icon: CursorIcon,
    /// 拖选时最近一次因鼠标超出编辑区上下边缘而滚动的时刻，用于按经过的时间计算滚动量
    /// （见 `scroll_while_dragging`）；鼠标不在边缘之外时为 `None`。
    pub(in crate::app) drag_scrolled_at: Option<Instant>,
    /// 鼠标是否在窗口内：移进来时置位，离开窗口时清除。
    pub(in crate::app) cursor_inside: bool,
}
