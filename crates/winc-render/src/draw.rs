//! 绘制：像素画布，以及在它上面画矩形、文字、图片的一帧。
//!
//! `canvas` 负责单个图形的混合与抗锯齿，`frame` 在上面加上裁剪、整体不透明度和常用的绘制命令。

use super::*;
pub(crate) mod canvas;
pub(crate) mod frame;
