//! 各平台安装包的 PNG、ICO 图标，均从应用内同一份 SVG 渲染。

use std::path::Path;

use crate::root;

/// 在 `out` 中生成 `winc.png`（256 像素）和含四种尺寸的 `winc.ico`。
///
/// # Errors
///
/// 读取、解析 SVG 或写入图标失败时返回原因。
pub fn write(out: &Path) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let text =
        std::fs::read_to_string(root().join("crates/winc-app/assets/icon.svg")).map_err(io)?;
    let picture = winc_preview::svg::load(&text, None, None)?;
    std::fs::create_dir_all(out).map_err(io)?;
    let mut images = Vec::new();
    for size in [16u32, 32, 48, 256] {
        let pixels = picture.render(size, size);
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, size, size);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .and_then(|mut writer| writer.write_image_data(&pixels.rgba))
                .map_err(|e| e.to_string())?;
        }
        if size == 256 {
            std::fs::write(out.join("winc.png"), &bytes).map_err(io)?;
        }
        images.push((size, bytes));
    }
    // ICO 目录：6 字节文件头（保留 0、类型 1、图像数 4），之后每项 16 字节；
    // 第一项图像数据的偏移等于头长加上目录项总长。
    let mut ico = vec![0, 0, 1, 0, 4, 0];
    let mut offset = 6 + 16 * 4;
    for (size, bytes) in &images {
        // ICO 的宽高字段各 1 字节，用 0 表示 256 像素。
        let dimension = if *size == 256 { 0 } else { *size as u8 };
        ico.extend([dimension, dimension, 0, 0]);
        ico.extend(1u16.to_le_bytes());
        ico.extend(32u16.to_le_bytes());
        ico.extend((bytes.len() as u32).to_le_bytes());
        ico.extend((offset as u32).to_le_bytes());
        offset += bytes.len();
    }
    for (_, bytes) in images {
        ico.extend(bytes);
    }
    std::fs::write(out.join("winc.ico"), ico).map_err(io)
}
