//! 终端仿真的性能测量：统计输入到终端仿真后每秒处理的字节数，以及按软换行重排回滚缓冲的
//! 耗时（重排是改尺寸里最重的一步）。输总量默认 100 MB，可用环境变量 `WINC_BENCH_MB` 调整。
//!
//! 不在平时的测试中运行：
//!
//! ```text
//! cargo test --release -p winc-pty --test throughput -- --ignored --nocapture
//! ```

use std::time::Instant;

use winc_pty::Terminal;
use winc_pty::protocol::Size;

/// 生成一段类似编译输出的字节样本：带颜色的前缀、路径、中文和长短不一的行。
fn sample() -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..2000 {
        match i % 4 {
            0 => out.extend_from_slice(
                format!("\x1b[1;32m   Compiling\x1b[0m winc-core v0.1.{i} (/Users/me/winc/crates/winc-core)\r\n")
                    .as_bytes(),
            ),
            1 => out.extend_from_slice(
                format!("\x1b[33mwarning\x1b[0m: 未使用的变量 `x{i}`，位于 src/lib.rs:{i}:9\r\n").as_bytes(),
            ),
            2 => out.extend_from_slice(b"plain ascii line with some words in it and a number 1234567890\r\n"),
            _ => out.extend_from_slice(&[b'x'; 200]),
        }
    }
    out
}

#[test]
#[ignore]
fn throughput() {
    let chunk = sample();
    // 输总量默认 100 MB；采样分析时用 `WINC_BENCH_MB` 调大，运行时间相应变长。
    let total = std::env::var("WINC_BENCH_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        << 20;
    let mut t = Terminal::new(200, 50, 10_000);

    let start = Instant::now();
    let mut fed = 0;
    let mut frames = 0;
    while fed < total {
        for piece in chunk.chunks(64 << 10) {
            t.feed(piece);
            fed += piece.len();
        }
        // 主程序按刷新率请求帧，这里每喂一段取一帧。
        if t.take_frame().is_some() {
            frames += 1;
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{:.0} MB 用了 {:.2} s，{:.0} MB/s，{frames} 帧",
        fed as f64 / 1048576.0,
        secs,
        fed as f64 / 1048576.0 / secs
    );
}

/// 回滚缓冲装满 `lines` 行后交替改宽度，每次重排的耗时取中位数和最大值（单位毫秒）。
///
/// 宽度在 200 列和 160 列之间交替：样本中 200 个 `x` 的行在 160 列时需要折行。
#[test]
#[ignore]
fn reflow() {
    let chunk = sample();
    for lines in [10_000, 100_000] {
        let mut t = Terminal::new(200, 50, lines);
        for _ in 0..lines / 2000 + 1 {
            t.feed(&chunk);
        }
        let mut times = Vec::new();
        for i in 0..20 {
            let cols = if i % 2 == 0 { 160 } else { 200 };
            let start = Instant::now();
            t.resize(Size {
                cols,
                rows: 50,
                width: cols * 16,
                height: 50 * 32,
            });
            times.push(start.elapsed());
        }
        times.sort();
        println!(
            "{lines} 行回滚缓冲改宽度：中位数 {:.1} ms，最大 {:.1} ms",
            times[10].as_secs_f64() * 1000.0,
            times[19].as_secs_f64() * 1000.0
        );
    }
}
