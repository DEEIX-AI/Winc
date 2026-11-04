//! `pasteboard` 的测试：独立的剪贴板中，只有 TIFF 时能转换成 PNG、已有 PNG 时原样取出，以及
//! 代码发出的每条 `AppKit` 消息都存在且参数编码一致。

use super::*;
use crate::macos::tests::method_encoding;

/// 新建一个独立的剪贴板（不是系统剪贴板），用完调用 `releaseGlobally` 释放。
fn private_board() -> Retained<AnyObject> {
    // SAFETY: `pasteboardWithUniqueName` 是公开的类方法，返回自动释放的对象，
    // `Retained::retain` 持有时取得所有权。
    unsafe {
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
        Retained::retain(board).unwrap()
    }
}

/// 把 `bytes` 以类型 `kind` 放到剪贴板 `board` 中（先清空）。
fn put(board: &AnyObject, kind: &str, bytes: &[u8]) {
    let kind = ns_string(kind).unwrap();
    // SAFETY: `clearContents`、`dataWithBytes:length:`、`setData:forType:` 是公开 API；
    // `bytes` 在调用期间有效，NSData 会复制一次。
    unsafe {
        let _: isize = msg_send![board, clearContents];
        let data: *mut AnyObject = msg_send![
            class!(NSData),
            dataWithBytes: bytes.as_ptr().cast::<std::ffi::c_void>(),
            length: bytes.len()
        ];
        let ok: bool = msg_send![board, setData: data, forType: &*kind];
        assert!(ok);
    }
}

/// 返回一张 2×2 的 TIFF 图片。
fn tiff() -> Vec<u8> {
    let space = ns_string("NSDeviceRGBColorSpace").unwrap();
    // SAFETY: `initWithBitmapDataPlanes:...` 在数据平面为空时自行分配像素，返回 +1 的对象，
    // 交给 `Retained` 管理；`TIFFRepresentation` 返回自动释放的 NSData，本块内拷贝。
    unsafe {
        let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: *mut AnyObject = msg_send![
            rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
            pixelsWide: 2isize,
            pixelsHigh: 2isize,
            bitsPerSample: 8isize,
            samplesPerPixel: 4isize,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: &*space,
            bytesPerRow: 0isize,
            bitsPerPixel: 0isize
        ];
        let rep = Retained::from_raw(rep).unwrap();
        let data: *mut AnyObject = msg_send![&*rep, TIFFRepresentation];
        data_bytes(data.as_ref().unwrap())
    }
}

/// 测试进程没有运行 `NSApplication`，第一次连接剪贴板服务和生成 TIFF 要十几秒；读取和转换
/// 本身约 1 ms。
///
/// 用独立的剪贴板，不动用户的系统剪贴板。
#[test]
fn images_are_read_as_png_and_tiff_is_converted() {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let board = private_board();

    // SAFETY: `board` 由 `Retained` 持有，在本测试中一直有效。
    unsafe {
        // 空剪贴板：没有图片。
        let _: isize = msg_send![&*board, clearContents];
        assert!(!has_image_on(&board));
        assert_eq!(png_on(&board), None);

        // 只有 TIFF：转换成 PNG。
        put(&board, TIFF, &tiff());
        assert!(has_image_on(&board));
        let png = png_on(&board).expect("TIFF 转换成 PNG");
        assert!(png.starts_with(SIGNATURE), "{:?}", &png[..8.min(png.len())]);

        // 有 PNG：原样取出，不重新编码。
        put(&board, PNG, &png);
        assert_eq!(png_on(&board).as_deref(), Some(png.as_slice()));

        let _: () = msg_send![&*board, releaseGlobally];
    }
}

#[test]
fn messages_match_the_appkit_declarations() {
    assert_eq!(
        method_encoding(c"NSPasteboard", sel!(generalPasteboard), false),
        "@@:"
    );
    assert_eq!(
        method_encoding(c"NSPasteboard", sel!(dataForType:), true),
        "@@:@"
    );
    assert_eq!(method_encoding(c"NSPasteboard", sel!(types), true), "@@:");
    assert_eq!(
        method_encoding(c"NSBitmapImageRep", sel!(imageRepWithData:), false),
        "@@:@"
    );
    assert_eq!(
        method_encoding(
            c"NSBitmapImageRep",
            sel!(representationUsingType:properties:),
            true
        ),
        "@@:Q@"
    );
    assert_eq!(
        method_encoding(c"NSArray", sel!(containsObject:), true),
        "B@:@"
    );
    assert_eq!(method_encoding(c"NSData", sel!(length), true), "Q@:");
    assert_eq!(method_encoding(c"NSData", sel!(bytes), true), "r^v@:");
}
