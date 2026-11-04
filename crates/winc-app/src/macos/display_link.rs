//! 按屏幕刷新的节奏绘制动画。
//!
//! 窗口所在的显示器每刷新一次（60 Hz 约 16.7 ms，120 Hz 的 `ProMotion` 约 8.3 ms），系统调用一次
//! [`DisplayLink`] 的回调，回调请求重绘：这次刷新之后绘制、下一次刷新时显示。不使用计时器：它总
//! 略晚于预定时刻唤醒，60 Hz 屏幕上相邻两帧实际隔 17.4 ms，每二十帧左右就有一次刷新没有新的帧。
//! `NSView` 的 `displayLinkWithTarget:selector:`（macOS 14 起）跟随窗口所在屏幕的刷新率，更早的
//! 系统退回计时器。

use std::sync::{Arc, Mutex, OnceLock};

use objc2::encode::{Encode, Encoding};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use winit::window::Window;

/// 每次刷新时调用的回调（请求重绘本窗口）。
type Tick = Arc<dyn Fn() + Send + Sync>;

/// 各个显示链接的回调，按接收回调的目标对象（每个窗口一个）的地址找：一个进程里有几个窗口，
/// 每个窗口的显示链接各叫各的窗口重画。
static TICKS: Mutex<Vec<(usize, Tick)>> = Mutex::new(Vec::new());

/// 实现 `- (void)wincFrame:(CADisplayLink *)link`：屏幕每次刷新时推进动画。
extern "C-unwind" fn frame(this: *mut AnyObject, _cmd: Sel, _link: *mut AnyObject) {
    #[cfg(test)]
    tests::FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // 先取出回调、放开锁再调：回调里请求重画，不会回到这里，但不必在锁里做事。
    let tick = TICKS.lock().ok().and_then(|ticks| {
        let found = ticks.iter().find(|(target, _)| *target == this as usize);
        found.map(|(_, tick)| Arc::clone(tick))
    });
    if let Some(tick) = tick {
        tick();
    }
}

/// `CAFrameRateRange`：最低、最高和期望的刷新率（Hz）。
#[repr(C)]
#[derive(Clone, Copy)]
struct FrameRateRange {
    /// 最低刷新率（Hz）。
    minimum: f32,
    /// 最高刷新率（Hz）。
    maximum: f32,
    /// 期望的刷新率（Hz）。
    preferred: f32,
}

// SAFETY: 编码与 QuartzCore 中 `CAFrameRateRange` 的定义一致，即三个 `float`；本结构体为
// `#[repr(C)]`，字段顺序和类型与之相同。
unsafe impl Encode for FrameRateRange {
    const ENCODING: Encoding = Encoding::Struct(
        "CAFrameRateRange",
        &[Encoding::Float, Encoding::Float, Encoding::Float],
    );
}

/// 显示链接：显示器每次刷新时唤醒界面；平时暂停，有动画时才运行。
pub struct DisplayLink {
    /// 底层的 `CADisplayLink` 对象。
    link: Retained<AnyObject>,
    /// 接收回调的目标对象的地址，在 [`TICKS`] 里找本链接的回调用。
    target: usize,
    /// 是否正在运行（未暂停）。
    running: bool,
}

impl DisplayLink {
    /// 为 `window` 创建一个处于暂停状态的显示链接，每次刷新调用 `tick`。
    ///
    /// 系统没有 `displayLinkWithTarget:selector:`（macOS 14 以下）时返回 `None`。
    pub fn new(window: &Window, tick: impl Fn() + Send + Sync + 'static) -> Option<DisplayLink> {
        let view = super::ns_view(window)?;
        let selector = sel!(displayLinkWithTarget:selector:);
        // SAFETY: `view` 是有效的 NSView；`respondsToSelector:` 是 NSObject 的公开方法，参数
        // 是有效的 selector。
        let available: bool = unsafe { msg_send![view, respondsToSelector: selector] };
        if !available {
            return None;
        }

        // SAFETY: 本块用到的都是公开 API，在主线程上调用。目标对象是只添加了一个方法的
        // `NSObject` 子类，方法签名与类型编码一致（返回 void，参数为 self、_cmd、link）。
        // `CADisplayLink` 强引用目标，`new` 得到的那一次引用交给 `Retained`，在函数末尾释放。
        // 显示链接是自动释放的对象，`Retained::retain` 持有一次，使其继续存活。
        unsafe {
            let target: Retained<AnyObject> = msg_send![target_class()?, new];
            let link: *mut AnyObject =
                msg_send![view, displayLinkWithTarget: &*target, selector: sel!(wincFrame:)];
            let link = Retained::retain(link)?;
            let key = Retained::as_ptr(&target) as usize;
            TICKS.lock().ok()?.push((key, Arc::new(tick)));

            // ProMotion 屏幕默认不一定使用最高刷新率，因此声明期望的范围：60 到 120 Hz，期望
            // 120 Hz。
            let range = FrameRateRange {
                minimum: 60.0,
                maximum: 120.0,
                preferred: 120.0,
            };
            let _: () = msg_send![&*link, setPreferredFrameRateRange: range];
            let _: () = msg_send![&*link, setPaused: Bool::YES];

            let run_loop: *mut AnyObject = msg_send![class!(NSRunLoop), mainRunLoop];
            schedule(&link, run_loop);

            Some(DisplayLink {
                link,
                target: key,
                running: false,
            })
        }
    }

    /// 设置显示链接是否运行：有动画时运行，没有动画时暂停，避免无谓地唤醒界面。
    pub fn set_running(&mut self, running: bool) {
        if running != self.running {
            self.running = running;
            // SAFETY: `self.link` 是有效的 `CADisplayLink`；`setPaused:` 是其公开方法，调用方在
            // 主线程上调用。
            unsafe {
                let _: () = msg_send![&*self.link, setPaused: Bool::new(!running)];
            }
        }
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        // SAFETY: `self.link` 是有效的 `CADisplayLink`；`invalidate` 把它从运行循环中移除，并
        // 释放它持有的目标。
        unsafe {
            let _: () = msg_send![&*self.link, invalidate];
        }
        if let Ok(mut ticks) = TICKS.lock() {
            ticks.retain(|(target, _)| *target != self.target);
        }
    }
}

/// 把显示链接加入 `run_loop` 的「通用模式」。
///
/// 拖动窗口、按住菜单时，主线程的运行循环处于其他模式，动画仍需继续，因此加在通用模式中。
///
/// # Safety
///
/// `link` 必须是有效的 `CADisplayLink`，`run_loop` 指向有效的 `NSRunLoop`。
unsafe fn schedule(link: &AnyObject, run_loop: *mut AnyObject) {
    // SAFETY: 由调用方保证 `link` 指向有效的 CADisplayLink、`run_loop` 指向有效的 NSRunLoop；
    // `NSRunLoopCommonModes` 是 Foundation 导出的常量，在进程内始终有效；本函数只在主线程调用。
    unsafe {
        let _: () = msg_send![link, addToRunLoop: run_loop, forMode: NSRunLoopCommonModes];
    }
}

/// 返回接收显示链接回调的类：`NSObject` 的子类，只添加一个方法 `wincFrame:`；类注册失败时
/// 返回 `None`。
fn target_class() -> Option<&'static AnyClass> {
    static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"WincFrameTarget", class!(NSObject))?;
        type Frame = extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);
        // SAFETY: 方法的签名与 `Frame` 一致（返回 void，参数为 self、_cmd、link）。
        unsafe { builder.add_method(sel!(wincFrame:), frame as Frame) };
        Some(builder.register())
    })
}

#[link(name = "Foundation", kind = "framework")]
// SAFETY: 声明与 Foundation 导出的符号一致：`NSRunLoopCommonModes` 是一个指向 NSString 的全局
// 变量，由框架在链接时提供，在进程整个生命周期内有效。
unsafe extern "C" {
    /// Foundation 导出的通用模式常量。
    ///
    /// 运行循环按指针识别该常量；若另外创建一个内容相同的字符串
    /// 作为模式，加入的显示链接永远不会被调用。
    static NSRunLoopCommonModes: &'static AnyObject;
}

#[cfg(test)]
mod tests;
