//! 剪贴板中的图片：取出 PNG 数据，只有 TIFF 时由系统转换成 PNG。
//!
//! 文本和文件列表由 arboard 读写；图片不经过 arboard，取出的 PNG 原样存盘，不解码也不重新
//! 编码，也就不需要额外的图片编解码库。

use super::files::ns_string;
use super::*;

/// 剪贴板中 PNG 图片的类型标识（UTI `public.png`）。
///
/// 截图和网页中复制的图片通常带有这种格式。
const PNG: &str = "public.png";

/// 剪贴板中 TIFF 图片的类型标识（UTI `public.tiff`）。
///
/// 有的程序复制图片时只提供这种格式，需要由系统转换成 PNG。
const TIFF: &str = "public.tiff";

/// 位图写成 PNG 时 `representationUsingType:` 要求的文件类型（`NSBitmapImageFileTypePNG`）。
const BITMAP_PNG: usize = 4;

/// 返回剪贴板中的图片，编码为 PNG；剪贴板中没有图片时返回 `None`。
///
/// 有 PNG 数据时原样取出；只有 TIFF 时用 `NSBitmapImageRep` 转换成 PNG（多页的 TIFF 取第一页）。
/// 测试中由 `App` 上的字段代替剪贴板，不调用本函数。
#[cfg_attr(test, allow(dead_code))]
pub fn pasteboard_png() -> Option<Vec<u8>> {
    // SAFETY: `general` 返回的是进程内共享、生命周期与进程相同的 NSPasteboard，不归调用方
    // 所有，不释放；指针非空时有效，满足 `png_on` 要求的前提。
    unsafe { png_on(general()?) }
}

/// 剪贴板中是否有图片（PNG 或 TIFF），不取出数据。
#[cfg_attr(test, allow(dead_code))]
pub fn pasteboard_has_image() -> bool {
    // SAFETY: `general` 返回的仍是同一个共享 NSPasteboard，在进程整个生命周期内有效且不归
    // 调用方所有；它的有效指针满足 `has_image_on` 要求的前提。
    unsafe { general().is_some_and(|board| has_image_on(board)) }
}

/// 返回系统剪贴板（`NSPasteboard` 的 `generalPasteboard`）。
///
/// # Safety
///
/// 返回的引用指向系统共享的对象，在进程生命周期内有效；调用方不得释放它。
#[cfg_attr(test, allow(dead_code))]
unsafe fn general() -> Option<&'static AnyObject> {
    // SAFETY: `generalPasteboard` 是公开的类方法，返回进程内共享、一直存在的对象。
    unsafe {
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        board.as_ref()
    }
}

/// 返回剪贴板 `board` 中的图片，编码为 PNG；没有图片时返回 `None`。
///
/// # Safety
///
/// `board` 必须指向有效的 `NSPasteboard`，并在调用期间保持存活。
unsafe fn png_on(board: &AnyObject) -> Option<Vec<u8>> {
    let png_type = ns_string(PNG)?;
    let tiff_type = ns_string(TIFF)?;
    // SAFETY: `dataForType:`、`imageRepWithData:`、`representationUsingType:properties:` 和
    // `dictionary` 都是公开 API，参数和返回值的类型与声明一致（见测试）。类型标识由 `Retained`
    // 持有，在本块内有效；返回的对象由自动释放池管理，在当前自动释放池排空前有效，本块内即
    // 拷贝出数据。
    unsafe {
        let png: *mut AnyObject = msg_send![board, dataForType: &*png_type];
        if let Some(png) = png.as_ref() {
            return Some(data_bytes(png));
        }

        let tiff: *mut AnyObject = msg_send![board, dataForType: &*tiff_type];
        let tiff = tiff.as_ref()?;
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        let rep = rep.as_ref()?;
        let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        let png: *mut AnyObject =
            msg_send![rep, representationUsingType: BITMAP_PNG, properties: properties];
        png.as_ref().map(|png| data_bytes(png))
    }
}

/// 剪贴板 `board` 中是否有图片（PNG 或 TIFF）。
///
/// # Safety
///
/// `board` 必须指向有效的 `NSPasteboard`，并在调用期间保持存活。
unsafe fn has_image_on(board: &AnyObject) -> bool {
    let (Some(png), Some(tiff)) = (ns_string(PNG), ns_string(TIFF)) else {
        return false;
    };
    // SAFETY: `types` 和 `containsObject:` 是公开 API；`types` 返回自动释放的 NSArray 或 nil，
    // 本块内使用；类型标识由 `Retained` 持有。
    unsafe {
        let types: *mut AnyObject = msg_send![board, types];
        let Some(types) = types.as_ref() else {
            return false;
        };
        [png, tiff].iter().any(|kind| {
            let has: bool = msg_send![types, containsObject: &**kind];
            has
        })
    }
}

/// 拷贝 `NSData` 中的字节。
///
/// # Safety
///
/// `data` 必须指向有效的 `NSData` 对象，并在调用期间保持存活。
unsafe fn data_bytes(data: &AnyObject) -> Vec<u8> {
    // SAFETY: 由调用方保证 `data` 是有效的 NSData；`bytes` 指向的 `length` 个字节在 `data`
    // 存活期间有效，此处立即拷贝。长度为 0 时 `bytes` 可能是空指针，不读取。
    unsafe {
        let length: usize = msg_send![data, length];
        let bytes: *const std::ffi::c_void = msg_send![data, bytes];
        if bytes.is_null() || length == 0 {
            return Vec::new();
        }
        std::slice::from_raw_parts(bytes.cast::<u8>(), length).to_vec()
    }
}

#[cfg(test)]
mod tests;
