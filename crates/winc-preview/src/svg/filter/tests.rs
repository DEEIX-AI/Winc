//! 滤镜原语的测试：膨胀与腐蚀的结果必须与逐像素扫描窗口的朴素实现相同。

use super::*;

/// 逐像素扫描窗口的朴素实现，结果必须与 [`morphology`] 相同。
fn naive(img: &Image, dilate: bool, rx: usize, ry: usize) -> Image {
    let mut out = img.clone();
    for (r, horizontal) in [(rx, true), (ry, false)] {
        if r == 0 {
            continue;
        }
        let src = out.clone();
        for y in 0..img.h {
            for x in 0..img.w {
                let mut acc = if dilate { [0.0f32; 4] } else { [1.0f32; 4] };
                for k in 0..=2 * r {
                    let (sx, sy) = if horizontal {
                        (x as isize + k as isize - r as isize, y as isize)
                    } else {
                        (x as isize, y as isize + k as isize - r as isize)
                    };
                    let outside = sx < 0 || sy < 0 || sx >= img.w as isize || sy >= img.h as isize;
                    let p = if outside {
                        [0.0; 4]
                    } else {
                        src.px[sy as usize * img.w + sx as usize]
                    };
                    for c in 0..4 {
                        acc[c] = if dilate {
                            acc[c].max(p[c])
                        } else {
                            acc[c].min(p[c])
                        };
                    }
                }
                out.px[y * img.w + x] = acc;
            }
        }
    }
    out
}

#[test]
fn morphology_matches_scanning_every_window() {
    // 13 × 7 的伪随机图像：宽和高互质，取模 17 后各通道覆盖多个取值，便于暴露窗口边界错误。
    let (w, h) = (13, 7);
    let mut img = Image::empty(w, h, false);
    for (i, p) in img.px.iter_mut().enumerate() {
        let v = |k: usize| ((i * 37 + k * 11) % 17) as f32 / 16.0;
        *p = [v(0), v(1), v(2), v(3)];
    }
    for dilate in [false, true] {
        for (rx, ry) in [(0, 0), (1, 0), (0, 2), (2, 3), (5, 1), (6, 6), (12, 20)] {
            let fast = morphology(&img, dilate, rx, ry);
            let slow = naive(&img, dilate, rx.min(w), ry.min(h));
            assert!(fast.px == slow.px, "dilate={dilate} r=({rx}, {ry})");
        }
    }
}
