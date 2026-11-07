//! 功能对话框（新建工作树、新建分支）的几何：窗口正中一张卡片，左边一窄列右对齐的名称，右边是
//! 输入框；选项是勾选框，与输入框左对齐；卡片底部右对齐「取消」「创建」两个按钮。
//!
//! 外框沿用设置浮层的尺寸：[`SETTINGS_HEADER`] 高的标题行和关闭按钮，左右、底下留
//! [`SETTINGS_PAD`]；框 [`DIALOG_FIELD_H`] 高（与提交信息框、侧栏搜索框相同），按钮
//! [`SETTINGS_CONTROL_H`] 高（与「提交」按钮相同）。名称列取 [`DIALOG_LABEL_W`]。
//!
//! `git`、`ssh` 的输入请求使用 [`DialogLayout::question`]：说明文字占整行，其下是只读信息和输入框。

use super::*;

/// 说明文字的行高（逻辑像素）。
pub const QUESTION_LINE_H: f32 = 18.0;
/// 询问对话框的宽度（逻辑像素）：等宽小字号的 SHA-256 指纹（50 个字符）可以在一行内完整显示。
const QUESTION_W: f32 = 520.0;
/// 询问对话框的名称列宽度（逻辑像素）：容纳英文名称中最长的 `Fingerprint`。
const QUESTION_LABEL_W: f32 = 76.0;

/// 对话框里可以点的部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogPart {
    /// 右上角的关闭按钮。
    Close,
    /// 「取消」。
    Cancel,
    /// 主按钮（「创建」）。
    Confirm,
    /// 分支名的输入框；询问对话框中是第一个输入框。
    Branch,
    /// 登录对话框的密码框。
    Password,
    /// 「基于」的分支菜单。
    Base,
    /// 「位置」右端选目录的按钮。
    Change,
    /// 「复制被忽略的文件」（勾选框连同名称都能点）。
    Copy,
    /// 「在新窗口中打开」。
    NewWindow,
    /// 新建分支的「创建后签出」。
    Checkout,
}

/// 新建工作树对话框的整套几何；绘制、悬停和点击都从这里取位置。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DialogLayout {
    /// 卡片。
    pub card: Rect,
    /// 询问对话框的说明文字；没有说明时为空矩形。
    pub message: Rect,
    /// 询问对话框的只读信息行。
    pub values: Vec<Rect>,
    /// 登录对话框的密码框；其他对话框为空矩形。
    pub password: Rect,
    /// 顶部的标题行。
    pub header: Rect,
    /// 关闭按钮。
    pub close: Rect,
    /// 每行左边的名称（字在里面右对齐）：分支、基于，有「位置」一行时还有位置。
    pub labels: Vec<Rect>,
    /// 分支名的输入框。
    pub branch: Rect,
    /// 「基于」的分支菜单。
    pub base: Rect,
    /// 「位置」：只读的目录，和它右端选目录的按钮（新建工作树才有）。
    pub location: Option<(Rect, Rect)>,
    /// 勾选项，一项一行。
    pub options: Vec<(DialogPart, Rect)>,
    /// 「取消」。
    pub cancel: Rect,
    /// 主按钮。
    pub confirm: Rect,
}

impl DialogLayout {
    /// 计算对话框的几何：`window` 是窗口矩形，`location` 表示有没有「位置」一行，`options` 是
    /// 下面的勾选项。
    ///
    /// 卡片宽 [`DIALOG_W`]，窗口太窄时缩到窗口以内；高度按内容算，水平居中。竖直方向偏上：
    /// 顶边距窗口上边缘为剩余空间高度的 42%，使视觉重心落在窗口中部。
    pub fn compute(window: Rect, location: bool, options: &[DialogPart]) -> Self {
        let w = DIALOG_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let inner = (w - SETTINGS_PAD * 2.0).max(0.0);
        // 先按卡片左上角在原点排好，最后整体平移到窗口里。
        let x = SETTINGS_PAD;
        let cx = x + DIALOG_LABEL_W + DIALOG_LABEL_GAP;
        let cw = (inner - DIALOG_LABEL_W - DIALOG_LABEL_GAP).max(0.0);
        let header = Rect::new(0.0, 0.0, w, SETTINGS_HEADER);
        let close = Rect::new(
            w - 12.0 - BUTTON,
            (SETTINGS_HEADER - BUTTON) / 2.0,
            BUTTON,
            BUTTON,
        );

        // 一行：左边名称，右边一个和提交信息框一样高的框，行与行隔 `DIALOG_GAP`。
        let mut y = header.bottom() + 4.0;
        let field = |y: &mut f32| {
            let label = Rect::new(x, *y, DIALOG_LABEL_W, DIALOG_FIELD_H);
            let control = Rect::new(cx, *y, cw, DIALOG_FIELD_H);
            *y += DIALOG_FIELD_H + DIALOG_GAP;
            (label, control)
        };
        let (branch_label, branch) = field(&mut y);
        let (base_label, base) = field(&mut y);
        let mut labels = vec![branch_label, base_label];
        let location = location.then(|| {
            let (label, location) = field(&mut y);
            labels.push(label);
            // 选目录的按钮在框里面的右端，上下各留 2。
            let change = Rect::new(
                location.right() - 2.0 - DIALOG_CHANGE_W,
                location.y + 2.0,
                DIALOG_CHANGE_W,
                location.h - 4.0,
            );
            (location, change)
        });

        // 勾选项排在框那一列，勾选框的左边和框的左边对齐。
        y += 2.0;
        let options: Vec<_> = (options.iter())
            .map(|&part| {
                let row = Rect::new(cx, y, cw, DIALOG_FIELD_H);
                y += row.h;
                (part, row)
            })
            .collect();

        y += 16.0;
        let confirm = Rect::new(
            x + inner - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let cancel = Rect::new(
            confirm.x - 8.0 - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let h = confirm.bottom() + SETTINGS_PAD;

        let room = (window.h - h).max(0.0);
        let (dx, dy) = (
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + room * 0.42).round(),
        );
        let at = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);

        DialogLayout {
            card: at(Rect::new(0.0, 0.0, w, h)),
            message: Rect::default(),
            values: Vec::new(),
            password: Rect::default(),
            header: at(header),
            close: at(close),
            labels: labels.into_iter().map(at).collect(),
            branch: at(branch),
            base: at(base),
            location: location.map(|(l, c)| (at(l), at(c))),
            options: options.into_iter().map(|(p, r)| (p, at(r))).collect(),
            cancel: at(cancel),
            confirm: at(confirm),
        }
    }

    /// 计算询问对话框的几何：`lines` 是说明文字的行数，`values` 是只读信息的行数，`inputs` 是输入框
    /// 的个数（最多 2 个）。
    ///
    /// 窗口太窄时卡片缩到窗口以内，位置规则与 [`DialogLayout::compute`] 相同。标签在 `labels` 中
    /// 按只读信息、输入框的顺序排列。
    pub fn question(window: Rect, lines: usize, values: usize, inputs: usize) -> Self {
        let w = QUESTION_W.min(window.w - SETTINGS_MARGIN * 2.0).max(0.0);
        let inner = (w - SETTINGS_PAD * 2.0).max(0.0);
        let x = SETTINGS_PAD;
        let cx = x + QUESTION_LABEL_W + DIALOG_LABEL_GAP;
        let cw = (inner - QUESTION_LABEL_W - DIALOG_LABEL_GAP).max(0.0);
        let header = Rect::new(0.0, 0.0, w, SETTINGS_HEADER);
        let close = Rect::new(
            w - 12.0 - BUTTON,
            (SETTINGS_HEADER - BUTTON) / 2.0,
            BUTTON,
            BUTTON,
        );
        let mut y = header.bottom() + 4.0;
        let message = Rect::new(x, y, inner, lines as f32 * QUESTION_LINE_H);
        if lines > 0 {
            y = message.bottom() + DIALOG_GAP + 2.0;
        }
        let mut labels = Vec::new();
        let mut row = |y: &mut f32| {
            labels.push(Rect::new(x, *y, QUESTION_LABEL_W, DIALOG_FIELD_H));
            let control = Rect::new(cx, *y, cw, DIALOG_FIELD_H);
            *y += DIALOG_FIELD_H + DIALOG_GAP;
            control
        };
        let rows: Vec<Rect> = (0..values).map(|_| row(&mut y)).collect();
        let fields: Vec<Rect> = (0..inputs).map(|_| row(&mut y)).collect();
        y += 6.0;
        let confirm = Rect::new(
            x + inner - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let cancel = Rect::new(
            confirm.x - 8.0 - DIALOG_BUTTON_W,
            y,
            DIALOG_BUTTON_W,
            SETTINGS_CONTROL_H,
        );
        let h = confirm.bottom() + SETTINGS_PAD;
        let room = (window.h - h).max(0.0);
        let (dx, dy) = (
            (window.x + (window.w - w) / 2.0).round(),
            (window.y + room * 0.42).round(),
        );
        let at = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.w, r.h);
        let field = |i: usize| fields.get(i).map_or(Rect::default(), |&r| at(r));
        DialogLayout {
            card: at(Rect::new(0.0, 0.0, w, h)),
            message: at(message),
            values: rows.iter().map(|&r| at(r)).collect(),
            password: field(1),
            header: at(header),
            close: at(close),
            labels: labels.into_iter().map(at).collect(),
            branch: field(0),
            cancel: at(cancel),
            confirm: at(confirm),
            ..DialogLayout::default()
        }
    }

    /// 返回 `(x, y)` 处可点击的部件；落在部件之外时返回 `None`。
    pub fn part_at(&self, x: f32, y: f32) -> Option<DialogPart> {
        let change = self
            .location
            .map(|(_, change)| (change, DialogPart::Change));
        let parts = [
            (self.close, DialogPart::Close),
            (self.cancel, DialogPart::Cancel),
            (self.confirm, DialogPart::Confirm),
            (self.branch, DialogPart::Branch),
            (self.password, DialogPart::Password),
            (self.base, DialogPart::Base),
        ];
        // 选目录的按钮叠在「位置」框里面，先匹配这一项。
        (change.into_iter().chain(parts))
            .chain(self.options.iter().map(|&(part, row)| (row, part)))
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, part)| part)
    }
}
