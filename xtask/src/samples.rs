//! 测试样本（`cargo xtask samples`）：大源文件、超大文件、压缩成一行的文件，生成到
//! `target/samples/`。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::root;

/// 返回测试样本的输出目录 `target/samples`。
pub fn samples_dir() -> PathBuf {
    root().join("target").join("samples")
}

/// 生成一个样本文件的函数。
type Writer = fn(&Path) -> std::io::Result<()>;

/// 生成全部测试样本，已存在的文件不覆盖。
///
/// # Errors
///
/// 创建目录、写入文件或读取文件元数据时返回对应的 `std::io::Error`。
pub fn samples() -> std::io::Result<()> {
    let dir = samples_dir();
    std::fs::create_dir_all(&dir)?;

    let jobs: [(&str, Writer); 5] = [
        ("medium.rs", |p| write_source(p, 24_000)),
        ("large.rs", |p| write_source(p, 100_000)),
        ("huge.txt", |p| write_huge(p, 100)),
        ("huge.rs", |p| write_source(p, 4_000_000)),
        ("minified.js", |p| write_minified(p, 5)),
    ];

    for (name, write) in jobs {
        let path = dir.join(name);
        if !path.exists() {
            write(&path)?;
        }
        println!(
            "{}  {:.1} MB",
            path.display(),
            std::fs::metadata(&path)?.len() as f64 / 1_048_576.0
        );
    }

    Ok(())
}

/// 确定性伪随机数发生器（线性同余），保证每次生成的样本内容一致。
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize % n.max(1)
    }
}

/// 写一个含模块文档、函数、结构体和注释的 Rust 文件，共 `lines` 行。
///
/// 内容混入中文注释和 emoji，覆盖字体回退与多字节字符。
fn write_source(path: &Path, lines: usize) -> std::io::Result<()> {
    let mut rng = Rng(0x5eed);
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);

    writeln!(out, "//! 自动生成的测试文件。")?;
    writeln!(out, "use std::collections::HashMap;")?;
    writeln!(out)?;

    let mut index = 0usize;

    while index < lines {
        let id = rng.below(1000);

        match rng.below(10) {
            0 => {
                writeln!(out, "/// 第 {index} 个条目：处理 {id} 号任务。")?;
                writeln!(
                    out,
                    "pub fn task_{index}(input: &str) -> Result<usize, Error> {{"
                )?;
                writeln!(out, "    let parsed = input.trim().parse::<usize>()?;")?;
                writeln!(out, "    let scaled = parsed * {id} + {};", rng.below(100))?;
                writeln!(
                    out,
                    "    // 这里的系数来自配置文件，不要随便改，否则会触发回归"
                )?;
                writeln!(out, "    let label = \"任务 {id} 已经完成 ✅\";")?;
                writeln!(out, "    if scaled > 1_000_000 {{")?;
                writeln!(
                    out,
                    "        return Err(Error::Overflow {{ value: scaled }});"
                )?;
                writeln!(out, "    }}")?;
                writeln!(out, "    Ok(scaled)")?;
                writeln!(out, "}}")?;
                index += 11;
            }
            1 => {
                writeln!(
                    out,
                    "struct Item{index} {{\n    name: String,\n    count: usize,\n    tags: Vec<String>,\n}}"
                )?;
                index += 5;
            }
            2 => {
                writeln!(out, "impl Item{index} {{")?;
                writeln!(out, "    fn new(name: &str) -> Self {{")?;
                writeln!(
                    out,
                    "        Self {{ name: name.to_owned(), count: {id}, tags: Vec::new() }}"
                )?;
                writeln!(out, "    }}\n}}")?;
                index += 5;
            }
            3 => {
                writeln!(out, "// TODO({id}): 这里的实现还没有覆盖边界情况")?;
                writeln!(
                    out,
                    "fn helper_{index}(map: &mut HashMap<String, usize>) {{"
                )?;
                writeln!(
                    out,
                    "    map.entry(\"键 {id}\".to_owned()).or_insert({index});\n}}"
                )?;
                index += 4;
            }
            _ => {
                writeln!(
                    out,
                    "    let value_{index} = compute({id}, {});",
                    rng.below(9999)
                )?;
                index += 1;
            }
        }
    }

    out.flush()
}

/// 写一个 `megabytes` MB 的纯文本文件，用于测量大文件的内存占用。
fn write_huge(path: &Path, megabytes: usize) -> std::io::Result<()> {
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let line = "0123456789 abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ 这是一行用来填充的文本，用于测试大文件的读取和内存占用。\n";
    let mut written = 0usize;

    while written < megabytes * 1_048_576 {
        out.write_all(line.as_bytes())?;
        written += line.len();
    }

    out.flush()
}

/// 写一个单行、`megabytes` MB 的文件，用于测量超长行的处理。
///
/// 内容形如压缩后的 JS：除末尾的换行外只有一行，标识符带编号。
fn write_minified(path: &Path, megabytes: usize) -> std::io::Result<()> {
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let mut written = 0usize;
    let mut i = 0usize;

    while written < megabytes * 1_048_576 {
        let chunk = format!("function f{i}(x){{return x*{i}+1;}}var v{i}=f{i}({i});");
        out.write_all(chunk.as_bytes())?;
        written += chunk.len();
        i += 1;
    }

    out.write_all(b"\n")?;
    out.flush()
}
