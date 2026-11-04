//! `display_link` 的测试：显示链接按应用的方式加入运行循环后确实被唤醒，以及代码发出的每条
//! `QuartzCore` 消息都存在且参数编码一致。

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::macos::tests::method_encoding;

/// 测试中回调被调用的次数。
pub(super) static FRAMES: AtomicUsize = AtomicUsize::new(0);

#[test]
fn a_scheduled_display_link_fires() {
    // 加入运行循环的方式不对（例如另外创建内容为 "kCFRunLoopCommonModes" 的字符串当作模式）时，
    // 显示链接一次也不会被调用，动画只画第一帧。这里改用屏幕的显示链接（不依赖窗口），
    // 以与应用相同的方式加入当前线程的运行循环，运行 0.3 秒后检查回调是否被调用。
    // SAFETY: 都是公开 API；`screen`、`link` 经判空后使用，`run_loop` 是当前线程的运行循环；
    // 显示链接用完后调用 `invalidate`。
    unsafe {
        let screen: *mut AnyObject = msg_send![class!(NSScreen), mainScreen];
        let Some(screen) = screen.as_ref() else {
            return;
        };
        if !msg_send![screen, respondsToSelector: sel!(displayLinkWithTarget:selector:)] {
            return;
        }
        let target: Retained<AnyObject> = msg_send![target_class().unwrap(), new];
        let link: *mut AnyObject =
            msg_send![screen, displayLinkWithTarget: &*target, selector: sel!(wincFrame:)];
        let link = Retained::retain(link).unwrap();
        let run_loop: *mut AnyObject = msg_send![class!(NSRunLoop), currentRunLoop];
        schedule(&link, run_loop);
        let before = FRAMES.load(Ordering::Relaxed);
        let until: *mut AnyObject = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.3f64];
        let _: () = msg_send![run_loop, runUntilDate: until];
        let _: () = msg_send![&*link, invalidate];
        assert!(FRAMES.load(Ordering::Relaxed) > before, "显示链接没有被叫");
    }
}

#[test]
fn display_link_messages_match_quartz_core() {
    // 只在 macOS 14 起有：更早的系统上 `new` 返回 `None`，不发这些消息。
    let Some(class) = AnyClass::get(c"CADisplayLink") else {
        return;
    };
    if class
        .instance_method(sel!(setPreferredFrameRateRange:))
        .is_none()
    {
        return;
    }
    assert_eq!(
        method_encoding(c"CADisplayLink", sel!(setPreferredFrameRateRange:), true),
        format!("v@:{}", FrameRateRange::ENCODING)
    );
    assert_eq!(
        method_encoding(c"CADisplayLink", sel!(setPaused:), true),
        format!("v@:{}", Bool::ENCODING)
    );
    assert_eq!(
        method_encoding(c"CADisplayLink", sel!(addToRunLoop:forMode:), true),
        "v@:@@"
    );
    assert_eq!(
        method_encoding(c"CADisplayLink", sel!(invalidate), true),
        "v@:"
    );
    assert_eq!(
        method_encoding(c"NSView", sel!(displayLinkWithTarget:selector:), true),
        "@@:@:"
    );

    // 接收回调的类：方法存在，未设置回调时调用也不会出错。
    let target = target_class().unwrap();
    assert!(target.instance_method(sel!(wincFrame:)).is_some());
    frame(std::ptr::null_mut(), sel!(wincFrame:), std::ptr::null_mut());
}
