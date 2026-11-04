//! 画布上按整段像素混合的循环。
//!
//! 这些循环没有分支，可以被向量化。x86-64 的基线指令集没有 32 位向量乘法，向量化的收益有限，
//! 因此同一份循环再按 AVX2 编译一份，运行时检测到 CPU 支持时使用；其他架构只有一份。

#![allow(unsafe_code)]

use super::{lerp, mix};

/// 定义一个按整段像素处理的函数，见模块说明。
macro_rules! wide {
    ($(#[$doc:meta])* fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $body:block) => {
        $(#[$doc])*
        pub(super) fn $name($($arg: $ty),*) {
            #[inline(always)]
            fn body($($arg: $ty),*) $body

            #[cfg(target_arch = "x86_64")]
            {
                #[target_feature(enable = "avx2")]
                fn avx2($($arg: $ty),*) {
                    body($($arg),*)
                }

                if std::is_x86_feature_detected!("avx2") {
                    // SAFETY: 上一行已确认 CPU 支持 AVX2。
                    return unsafe { avx2($($arg),*) };
                }
            }
            body($($arg),*)
        }
    };
}

wide! {
    /// 按透明度 `sa` 把 `color` 的颜色叠到 `row` 的每个像素上，见 [`mix`]。
    fn mix_span(row: &mut [u32], color: u32, sa: u32) {
        for p in row {
            *p = mix(*p, color, sa);
        }
    }
}

wide! {
    /// 让 `out` 的每个像素等于 `src` 的对应像素按权重 `a` 向 `color` 过渡的结果，见 [`lerp`]。
    fn lerp_to_color(out: &mut [u32], src: &[u32], color: u32, a: u32) {
        for (p, &s) in out.iter_mut().zip(src) {
            *p = lerp(s, color, a);
        }
    }
}

wide! {
    /// 让 `out` 的每个像素等于 `first` 的对应像素按权重 `a` 向 `second` 过渡的结果。
    fn lerp_rows(out: &mut [u32], first: &[u32], second: &[u32], a: u32) {
        for ((p, &f), &s) in out.iter_mut().zip(first).zip(second) {
            *p = lerp(f, s, a);
        }
    }
}

wide! {
    /// 把 `top` 的每个像素按权重 `a` 叠到 `row` 的对应像素上。
    fn lerp_onto(row: &mut [u32], top: &[u32], a: u32) {
        for (p, &t) in row.iter_mut().zip(top) {
            *p = lerp(*p, t, a);
        }
    }
}

wide! {
    /// 按各自的权重 `weights` 混合 `first` 和 `second` 的对应像素，再按权重 `a` 叠到 `row` 上。
    fn lerp_pairs_onto(
        row: &mut [u32],
        first: &[u32],
        second: &[u32],
        weights: &[u32],
        a: u32,
    ) {
        for (((p, &f), &s), &w) in row.iter_mut().zip(first).zip(second).zip(weights) {
            *p = lerp(*p, lerp(f, s, w), a);
        }
    }
}
