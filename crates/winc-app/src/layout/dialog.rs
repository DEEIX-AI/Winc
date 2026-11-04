//! 功能对话框（新建工作树、新建分支）的几何：窗口正中一张卡片，左边一窄列右对齐的名称，右边是
//! 输入框；选项是勾选框，与输入框左对齐；卡片底部右对齐「取消」「创建」两个按钮。
//!
//! 外框沿用设置浮层的尺寸：[`SETTINGS_HEADER`] 高的标题行和关闭按钮，左右、底下留
//! [`SETTINGS_PAD`]；框 [`DIALOG_FIELD_H`] 高（与提交信息框、侧栏搜索框相同），按钮
//! [`SETTINGS_CONTROL_H`] 高（与「提交」按钮相同）。名称列取 [`DIALOG_LABEL_W`]。

use super::*;

/// 对话框里可以点的部件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogPart {
    /// 右上角的关闭按钮。
    Close,
    /// 「取消」。
    Cancel,
    /// 主按钮（「创建」）。
    Confirm,
    /// 分支名的输入框。
    Branch,
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
            (self.base, DialogPart::Base),
        ];
        // 选目录的按钮叠在「位置」框里面，先匹配这一项。
        (change.into_iter().chain(parts))
            .chain(self.options.iter().map(|&(part, row)| (row, part)))
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, part)| part)
    }
}
