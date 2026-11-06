//! `dialogs` 的测试：图标像素写入位图后不变形，以及代码发出的每条 `AppKit` 消息都真实存在且
//! 参数编码与传入类型一致。

use super::*;
use crate::macos::tests::method_encoding;

#[test]
fn icon_images_keep_the_pixels_and_the_size() {
    // 2×1 像素当成 1×0.5 点：左边不透明的绿，右边全透明。
    let pixels = Image {
        pixels: vec![0xff2e_9e5b, 0x0000_0000],
        width: 2,
        height: 1,
    };
    let icon = MenuIcon {
        image: &pixels,
        size: (1.0, 0.5),
    };
    let image = icon_image(&icon).unwrap();

    // SAFETY: NSImage、NSBitmapImageRep、NSColor 的公开 API；返回值先判空再使用；四个输出
    // 参数指向存活的局部变量。
    unsafe {
        let size: CGSize = msg_send![&*image, size];
        assert_eq!((size.width, size.height), (1.0, 0.5));

        let reps: *mut AnyObject = msg_send![&*image, representations];
        let rep: *mut AnyObject = msg_send![reps.as_ref().unwrap(), firstObject];
        let rep = rep.as_ref().unwrap();
        let wide: isize = msg_send![rep, pixelsWide];
        assert_eq!(wide, 2);
        let pixel = |x: isize| {
            let color: *mut AnyObject = msg_send![rep, colorAtX: x, y: 0isize];
            let (mut r, mut g, mut b, mut a) = (0f64, 0f64, 0f64, 0f64);
            let () = msg_send![
                color.as_ref().unwrap(),
                getRed: &mut r,
                green: &mut g,
                blue: &mut b,
                alpha: &mut a
            ];
            [r, g, b, a].map(|v| (v * 255.0).round() as u8)
        };
        assert_eq!(pixel(0), [0x2e, 0x9e, 0x5b, 255]);
        assert_eq!(pixel(1)[3], 0);
    }
}

#[test]
fn panel_messages_match_appkit_signatures() {
    // 面板不能在测试中弹出（会显示在屏幕上，且必须在主线程），因此只核对发出的每条消息在
    // AppKit 中确实存在，且参数类型与传入的一致。
    let encoding = method_encoding;

    let bool_setter = "v@:B";
    for name in [
        sel!(setCanChooseFiles:),
        sel!(setCanChooseDirectories:),
        sel!(setAllowsMultipleSelection:),
    ] {
        assert_eq!(encoding(c"NSOpenPanel", name, true), bool_setter);
    }
    for name in [
        sel!(setPrompt:),
        sel!(setNameFieldLabel:),
        sel!(setNameFieldStringValue:),
        sel!(setDirectoryURL:),
    ] {
        assert_eq!(encoding(c"NSSavePanel", name, true), "v@:@");
    }
    assert_eq!(encoding(c"NSSavePanel", sel!(URL), true), "@@:");
    assert_eq!(
        encoding(
            c"NSSavePanel",
            sel!(beginSheetModalForWindow:completionHandler:),
            true
        ),
        "v@:@@?"
    );
    assert_eq!(encoding(c"NSOpenPanel", sel!(openPanel), false), "@@:");
    assert_eq!(encoding(c"NSSavePanel", sel!(savePanel), false), "@@:");
    assert_eq!(encoding(c"NSWindow", sel!(attachedSheet), true), "@@:");
    assert_eq!(
        encoding(c"NSURL", sel!(fileURLWithPath:isDirectory:), false),
        "@@:@B"
    );
    assert!(encoding(c"NSURL", sel!(fileSystemRepresentation), true).ends_with("*@:"));

    // 右键菜单。
    assert_eq!(
        encoding(c"NSMenu", sel!(setAutoenablesItems:), true),
        bool_setter
    );
    assert_eq!(encoding(c"NSMenu", sel!(addItem:), true), "v@:@");
    assert_eq!(encoding(c"NSMenu", sel!(setMinimumWidth:), true), "v@:d");
    assert_eq!(
        encoding(
            c"NSMenu",
            sel!(popUpMenuPositioningItem:atLocation:inView:),
            true
        ),
        "B@:@{CGPoint=dd}@"
    );
    assert_eq!(
        encoding(
            c"NSMenuItem",
            sel!(initWithTitle:action:keyEquivalent:),
            true
        ),
        "@@:@:@"
    );
    assert_eq!(
        encoding(c"NSMenuItem", sel!(setKeyEquivalentModifierMask:), true),
        "v@:Q"
    );
    assert_eq!(encoding(c"NSMenuItem", sel!(setTag:), true), "v@:q");
    assert_eq!(encoding(c"NSMenuItem", sel!(setState:), true), "v@:q");
    assert_eq!(encoding(c"NSMenuItem", sel!(tag), true), "q@:");
    assert_eq!(
        encoding(c"NSMenuItem", sel!(setEnabled:), true),
        bool_setter
    );
    assert_eq!(encoding(c"NSMenuItem", sel!(setTarget:), true), "v@:@");
    assert_eq!(encoding(c"NSMenuItem", sel!(separatorItem), false), "@@:");

    // 最近项目菜单：小字、图标、悬停提示。
    assert_eq!(encoding(c"NSMenuItem", sel!(setSubtitle:), true), "v@:@");
    assert_eq!(encoding(c"NSMenuItem", sel!(setImage:), true), "v@:@");
    assert_eq!(encoding(c"NSMenuItem", sel!(setToolTip:), true), "v@:@");
    assert_eq!(
        encoding(
            c"NSBitmapImageRep",
            sel!(initWithBitmapDataPlanes:pixelsWide:pixelsHigh:bitsPerSample:samplesPerPixel:hasAlpha:isPlanar:colorSpaceName:bytesPerRow:bitsPerPixel:),
            true
        ),
        "@@:^*qqqqBB@qq"
    );
    assert_eq!(encoding(c"NSBitmapImageRep", sel!(bitmapData), true), "*@:");
    assert_eq!(
        encoding(
            c"NSBitmapImageRep",
            sel!(bitmapImageRepByRetaggingWithColorSpace:),
            true
        ),
        "@@:@"
    );
    assert_eq!(
        encoding(c"NSColorSpace", sel!(sRGBColorSpace), false),
        "@@:"
    );
    assert_eq!(
        encoding(c"NSImage", sel!(initWithSize:), true),
        "@@:{CGSize=dd}"
    );
    assert_eq!(encoding(c"NSImage", sel!(addRepresentation:), true), "v@:@");

    // 确认框。
    for name in [sel!(setMessageText:), sel!(setInformativeText:)] {
        assert_eq!(encoding(c"NSAlert", name, true), "v@:@");
    }
    assert_eq!(
        encoding(c"NSAlert", sel!(addButtonWithTitle:), true),
        "@@:@"
    );
    assert_eq!(
        encoding(
            c"NSAlert",
            sel!(beginSheetModalForWindow:completionHandler:),
            true
        ),
        "v@:@@?"
    );
    assert_eq!(encoding(c"NSButton", sel!(setKeyEquivalent:), true), "v@:@");
    assert_eq!(
        encoding(c"NSAlert", sel!(setShowsSuppressionButton:), true),
        bool_setter
    );
    assert_eq!(encoding(c"NSAlert", sel!(suppressionButton), true), "@@:");
    assert_eq!(encoding(c"NSButton", sel!(setTitle:), true), "v@:@");
    assert_eq!(encoding(c"NSButton", sel!(state), true), "q@:");
    assert_eq!(
        encoding(c"NSButton", sel!(setKeyEquivalentModifierMask:), true),
        "v@:Q"
    );

    // 废纸篓。
    assert_eq!(encoding(c"NSURL", sel!(fileURLWithPath:), false), "@@:@");
    assert_eq!(
        encoding(
            c"NSFileManager",
            sel!(trashItemAtURL:resultingItemURL:error:),
            true
        ),
        "B@:@^@^@"
    );
    assert_eq!(
        encoding(c"NSError", sel!(localizedDescription), true),
        "@@:"
    );
}
