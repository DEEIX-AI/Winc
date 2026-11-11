//! 对照测试：读取录制文件，把 `vim` 的执行结果与 `winc-vim` 的结果比较。
//!
//! 录制文件（`fixtures/*.rec`）由 `cargo xtask vim-fixtures` 从同名的用例文件（`.cases`）生成，
//! 测试不需要 `vim`。命令的求值实现之前，只有 `parser.rec` 的结果参与比较：其中的用例只用到
//! 删除字符与移到行首，由本文件中的模型执行。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use winc_vim::fixture::{self, Case, File, Position, State};
use winc_vim::{Action, Body, Command, Mode, Motion, Outcome, Parser, notation};

/// 录制基准的版本号前缀。
const BASELINE: &str = "9.1.";

/// 返回用例文件与录制文件所在的目录。
fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// 读取并解析目录中扩展名为 `extension` 的全部文件，按文件名排序；返回主文件名与内容。
fn load(extension: &str) -> Vec<(String, File)> {
    let mut files: Vec<(String, File)> = std::fs::read_dir(dir())
        .expect("fixtures 目录存在")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap();
            let file = fixture::parse(&text).unwrap_or_else(|e| panic!("{name}.{extension}：{e}"));
            (name, file)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn every_case_file_has_an_up_to_date_recording() {
    let cases = load("cases");
    let recordings = load("rec");
    assert!(!cases.is_empty(), "至少有一个用例文件");
    let names = |files: &[(String, File)]| files.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&cases),
        names(&recordings),
        "用例文件与录制文件一一对应"
    );

    for ((name, source), (_, recorded)) in cases.iter().zip(&recordings) {
        let version = recorded.vim.as_deref().unwrap_or_default();
        assert!(
            version.starts_with(BASELINE),
            "{name}.rec 的版本是 {version:?}"
        );
        assert!(!recorded.cases.is_empty(), "{name}.rec 没有用例");
        assert!(
            recorded.cases.iter().all(|c| c.result.is_some()),
            "{name}.rec 缺少结果"
        );
        // 录制文件的输入与用例文件相同；不同时需要重新运行 `cargo xtask vim-fixtures`。
        let inputs: Vec<Case> = (recorded.cases.iter())
            .map(|case| Case {
                result: None,
                ..case.clone()
            })
            .collect();
        assert_eq!(
            inputs, source.cases,
            "{name}.rec 与 {name}.cases 的输入不一致"
        );
    }
}

/// 是否有字符位于 `text` 的位置 `at`；空行只有第 0 列算作有字符。
fn on_a_character(text: &[String], at: Position) -> bool {
    (text.get(at.line)).is_some_and(|line| at.col < line.chars().count().max(1))
}

#[test]
fn recorded_cases_are_well_formed() {
    for (name, file) in load("rec") {
        for case in &file.cases {
            let label = format!("{name}.rec 的用例 {}", case.name);
            // 按键写成规范的按键记法，避免记法写错时被当作普通字符送入。
            let keys = notation::parse(&case.keys);
            assert_eq!(notation::write(&keys), case.keys, "{label}");
            assert!(on_a_character(&case.text, case.cursor), "{label}");

            let result = case.result.as_ref().unwrap();
            assert!(on_a_character(&result.text, result.cursor), "{label}");
            for register in &result.registers {
                assert!(!register.lines.is_empty(), "{label}");
            }
        }
    }
}

/// 探针用例的模型：单行文本、光标，以及具名寄存器的内容。
struct Probe {
    /// 文本的字符。
    line: Vec<char>,
    /// 光标所在的字符下标。
    col: usize,
    /// 具名寄存器（小写字母）的内容。
    named: BTreeMap<char, String>,
}

impl Probe {
    /// 执行一条命令；只接受删除字符与移到行首。
    fn run(&mut self, command: Command) {
        match command.body {
            Body::Motion(movement) if movement.motion == Motion::LineStart => self.col = 0,
            Body::Action {
                action: Action::DeleteChar,
                ..
            } => {
                let count = command.count.map_or(1, |n| usize::try_from(n).unwrap());
                let end = self.line.len().min(self.col.saturating_add(count));
                let removed: String = self.line.drain(self.col..end).collect();
                if removed.is_empty() {
                    return;
                }
                self.col = self.col.min(self.line.len().saturating_sub(1));
                match command.register {
                    Some(name) if name.is_ascii_lowercase() => {
                        self.named.insert(name, removed);
                    }
                    Some(name) if name.is_ascii_uppercase() => {
                        let entry = self.named.entry(name.to_ascii_lowercase()).or_default();
                        entry.push_str(&removed);
                    }
                    _ => {}
                }
            }
            other => panic!("探针用例只能使用 x 与 0：{other:?}"),
        }
    }
}

/// 返回录制结果 `state` 中具名寄存器（小写字母）的内容。
fn named_registers(state: &State) -> BTreeMap<char, String> {
    (state.registers.iter())
        .filter(|register| register.name.is_ascii_lowercase())
        .map(|register| (register.name, register.lines.join("\n")))
        .collect()
}

#[test]
fn parser_probes_match_the_recording() {
    let recordings = load("rec");
    let (_, file) = (recordings.iter())
        .find(|(name, _)| name == "parser")
        .expect("parser.rec 存在");
    assert!(file.cases.len() >= 30);

    for case in &file.cases {
        let [line] = case.text.as_slice() else {
            panic!("探针用例 {} 的文本只能有一行", case.name);
        };
        let mut probe = Probe {
            line: line.chars().collect(),
            col: case.cursor.col,
            named: BTreeMap::new(),
        };
        // 按键出错之后余下的按键继续执行，与录制时相同。
        let mut parser = Parser::default();
        for key in notation::parse(&case.keys) {
            if let Outcome::Command(command) = parser.feed(Mode::Normal, key) {
                probe.run(command);
            }
        }

        let result = case.result.as_ref().unwrap();
        let text: String = probe.line.iter().collect();
        assert_eq!([text], result.text.as_slice(), "{}：文本", case.name);
        assert_eq!(probe.col, result.cursor.col, "{}：光标", case.name);
        assert_eq!(
            probe.named,
            named_registers(result),
            "{}：寄存器",
            case.name
        );
        assert!(
            parser.pending().is_empty(),
            "{}：按键序列结束时没有未完成的命令",
            case.name
        );
    }
}
