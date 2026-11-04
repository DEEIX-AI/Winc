//! macOS 上用系统的 `ImageIO` 解码要缩小的 JPEG：按目标大小直接解出缩小的图。
//!
//! zune-jpeg 只能先解出整张原图再缩小，一亿像素的照片要先占约 400 MB（RGBA8）；`ImageIO` 的
//! 缩略图（`CGImageSourceCreateThumbnailAtIndex`）在解码时按 DCT 缩放，内存和耗时都低一个数量级。
//!
//! 不按 EXIF 方向旋转，保持图片原来的像素方向；画进 sRGB 的位图，任何色彩空间（Display P3、
//! Adobe RGB、CMYK、灰度）都由 CoreGraphics 转成 sRGB。

#![allow(unsafe_code)] // ImageIO 没有 objc2 绑定，自己声明两个函数和几个键

use std::ptr::NonNull;

use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, kCGColorSpaceSRGB,
};

use super::Pixels;

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    /// 返回 `CGImageSourceRef`（按 Create 规则持有），失败时为空。
    fn CGImageSourceCreateWithData(
        data: &CFData,
        options: Option<&CFDictionary>,
    ) -> Option<NonNull<CFType>>;
    /// 返回 `CGImageRef`（按 Create 规则持有），失败时为空。
    fn CGImageSourceCreateThumbnailAtIndex(
        source: &CFType,
        index: usize,
        options: Option<&CFDictionary>,
    ) -> Option<NonNull<CGImage>>;
    static kCGImageSourceCreateThumbnailFromImageAlways: &'static CFString;
    static kCGImageSourceThumbnailMaxPixelSize: &'static CFString;
    static kCGImageSourceShouldCacheImmediately: &'static CFString;
}

/// 把 JPEG `bytes` 解成长边不超过 `max` 的图；`ImageIO` 解不出来时返回 `None`，由调用方改用
/// zune-jpeg。
pub(super) fn decode_scaled(bytes: &[u8], max: u32) -> Option<Pixels> {
    let data = CFData::from_bytes(bytes);
    // SAFETY: `data` 是有效的 CFData；返回值按 Create 规则持有，交给 `CFRetained` 释放。
    let source = unsafe { CFRetained::from_raw(CGImageSourceCreateWithData(&data, None)?) };

    let max = CFNumber::new_i32(i32::try_from(max).ok()?);
    let yes = CFBoolean::new(true);
    // SAFETY: 这三个键是 ImageIO 导出的常量字符串，进程存在期间一直有效。
    let keys: [&CFString; 3] = unsafe {
        [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceThumbnailMaxPixelSize,
            kCGImageSourceShouldCacheImmediately,
        ]
    };
    // 总是从原图生成（`FromImageAlways`），不用文件里嵌的小缩略图：那张往往只有 160 像素宽。
    let values: [&CFType; 3] = [yes.as_ref(), max.as_ref(), yes.as_ref()];
    let options = CFDictionary::from_slices(&keys, &values);
    // SAFETY: `source` 是有效的 `CGImageSourceRef`，`options` 是键为 CFString 的字典；
    // 返回值按 Create 规则持有。
    let image = unsafe {
        CFRetained::from_raw(CGImageSourceCreateThumbnailAtIndex(
            &source,
            0,
            Some(options.as_opaque()),
        )?)
    };

    let (width, height) = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
    if width == 0 || height == 0 {
        return None;
    }
    // 预览按 sRGB 显示：按图片自己的色彩空间画出来的数值不能直接当 sRGB 用，例如 iPhone 照片的
    // Display P3 会显得发灰。画进 sRGB 的位图时由 CoreGraphics 转换。
    // SAFETY: `kCGColorSpaceSRGB` 是 CoreGraphics 导出的常量字符串。
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    let mut rgba = vec![0u8; width.checked_mul(height)?.checked_mul(4)?];
    // SAFETY: `rgba` 有 `width × height × 4` 字节，每行 `width × 4` 字节，与传入的尺寸一致；
    // 每像素 R、G、B 和一个不用的字节（`NoneSkipLast`），每个分量 8 位。`rgba` 在 `context`
    // 释放之前一直存活且不被移动。
    let context = unsafe {
        CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            Some(&space),
            CGImageAlphaInfo::NoneSkipLast.0,
        )
    }?;
    let rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(width as f64, height as f64),
    );
    CGContext::draw_image(Some(&context), rect, Some(&image));
    CGContext::flush(Some(&context));
    drop(context);
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    Some(Pixels {
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        rgba,
    })
}
