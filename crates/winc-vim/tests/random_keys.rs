//! 随机按键测试：固定种子的随机按键序列直接驱动解析器，核对不 panic 与各项不变量。
//!
//! 序列由种子决定，失败时的信息含种子与步数，可以原样重现。按键大多取自命令表用到的按键，
//! 使序列经常构成完整的命令；其余是任意的字符、功能键与修饰键组合。

use winc_vim::command::ENTRIES;
use winc_vim::parser::MAX_COUNT;
use winc_vim::{Action, Body, Code, Command, Key, Mode, Mods, Motion, Named, Outcome, Parser};
use winc_vim::{Target, notation};

/// 伪随机数生成器（SplitMix64）：同一个种子给出同一个序列，不依赖平台。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 返回 `0..n` 之内的数；`n` 不为 0。
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

const NAMED: [Named; 16] = [
    Named::Esc,
    Named::Enter,
    Named::Tab,
    Named::Backspace,
    Named::Delete,
    Named::Insert,
    Named::Up,
    Named::Down,
    Named::Left,
    Named::Right,
    Named::Home,
    Named::End,
    Named::PageUp,
    Named::PageDown,
    Named::F(1),
    Named::F(35),
];

const MODES: [Mode; 4] = Mode::ALL;

/// 返回常用的按键：命令表中出现的全部按键、数字、寄存器名、功能键与若干非 ASCII 字符。
fn common_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = (ENTRIES.iter())
        .flat_map(|entry| notation::parse(entry.keys))
        .collect();
    keys.extend(
        "0123456789\"-_+*/:.%=#azAZ!@ \u{301}中é😀"
            .chars()
            .map(Key::char),
    );
    keys.extend(NAMED.map(Key::named));
    // 取消与次数的删除在序列中多出现一些。
    keys.extend([Key::named(Named::Esc), Key::named(Named::Delete)].repeat(4));
    keys
}

/// 返回一个随机的按键：多数取自 `common`，其余是任意的主体与修饰键。
fn random_key(rng: &mut Rng, common: &[Key]) -> Key {
    if rng.below(100) < 85 {
        return rng.pick(common);
    }
    let code = match rng.below(4) {
        0 => Code::Named(rng.pick(&NAMED)),
        1 => Code::Named(Named::F(u8::try_from(rng.below(256)).unwrap())),
        // 任意码位：包括控制字符与 ASCII 之外的字符；代理区不是字符，改用替换字符。
        2 => Code::Char(
            char::from_u32(u32::try_from(rng.below(0x11_0000)).unwrap()).unwrap_or('\u{fffd}'),
        ),
        _ => Code::Char(char::from(u8::try_from(rng.below(128)).unwrap())),
    };
    let mods = [Mods::CTRL, Mods::SHIFT, Mods::ALT, Mods::CMD]
        .into_iter()
        .filter(|_| rng.below(4) == 0)
        .fold(Mods::NONE, Mods::with);
    Key::new(code, mods)
}

/// 是否参数与命令相符：行内查找、替换字符、设置标记、插入寄存器带字符参数，其余命令不带。
fn argument_fits(command: &Command) -> bool {
    let find = |motion: Motion| {
        matches!(
            motion,
            Motion::FindForward | Motion::FindBackward | Motion::TillForward | Motion::TillBackward
        )
    };
    match command.body {
        Body::Motion(movement)
        | Body::Operator {
            target: Target::Motion(movement),
            ..
        } => find(movement.motion) == movement.arg.is_some(),
        Body::Action { action, arg } => {
            let expects = matches!(
                action,
                Action::ReplaceChar | Action::SetMark | Action::InsertRegister
            );
            expects == arg.is_some()
        }
        _ => true,
    }
}

/// 核对一条命令自身的不变量，以及与所在模式的关系。
fn check_command(mode: Mode, command: &Command, context: &str) {
    if let Some(count) = command.count {
        assert!((1..=MAX_COUNT).contains(&count), "{context}：次数 {count}");
    }
    if let Some(name) = command.register {
        assert!(
            name.is_ascii_alphanumeric() || "\"-_+*/:.%".contains(name),
            "{context}：寄存器 {name:?}"
        );
    }
    assert!(argument_fits(command), "{context}：参数与命令不符");

    match (mode, command.body) {
        (Mode::Insert, body) => {
            assert!(matches!(body, Body::Action { .. }), "{context}");
            assert_eq!((command.register, command.count), (None, None), "{context}");
        }
        (Mode::Visual, Body::Operator { target, force, .. }) => {
            assert_eq!((target, force), (Target::Selection, None), "{context}");
        }
        (_, Body::Operator { target, .. }) => assert_ne!(target, Target::Selection, "{context}"),
        (Mode::Normal | Mode::OperatorPending, Body::Object(_)) => {
            panic!("{context}：普通模式下不能单独使用文本对象")
        }
        _ => {}
    }
}

/// 以种子 `seed` 生成 `steps` 个按键驱动解析器，返回各类结果出现的次数。
///
/// 另有一个解析器在每条命令结束后重置：两者的结果必须始终相同，即命令结束后解析器不保留状态。
fn run(seed: u64, steps: usize, common: &[Key]) -> [usize; 6] {
    let mut rng = Rng(seed);
    let mut parser = Parser::default();
    let mut fresh = Parser::default();
    let mut mode = Mode::Normal;
    let mut seen = [0; 6];

    for step in 0..steps {
        if rng.below(50) == 0 {
            mode = rng.pick(&MODES);
        }
        let key = random_key(&mut rng, common);
        let context = format!("种子 {seed} 第 {step} 步，{mode:?} 模式，按键 {key}");

        let outcome = parser.feed(mode, key);
        assert_eq!(
            fresh.feed(mode, key),
            outcome,
            "{context}：重置后的解析器结果不同"
        );
        if outcome == Outcome::Pending {
            let pending = parser.pending();
            assert!(!pending.is_empty(), "{context}");
            assert_eq!(
                notation::parse(&notation::write(pending)),
                pending,
                "{context}"
            );
        } else {
            assert!(parser.pending().is_empty(), "{context}");
            assert!(!parser.is_operator_pending(), "{context}");
            fresh.reset();
        }
        if let Outcome::Command(command) = &outcome {
            check_command(mode, command, &context);
        }
        if mode == Mode::Insert {
            assert!(!parser.is_operator_pending(), "{context}");
        }

        seen[match outcome {
            Outcome::Pending => 0,
            Outcome::Command(_) => 1,
            Outcome::Unsupported => 2,
            Outcome::Invalid => 3,
            Outcome::Cancelled => 4,
            Outcome::Unbound => 5,
        }] += 1;
    }
    seen
}

#[test]
fn random_keys_never_break_the_parser() {
    let common = common_keys();
    let mut total = [0; 6];
    for seed in 0..200 {
        let seen = run(seed, 2_000, &common);
        // 同一个种子给出同一个结果。
        if seed < 4 {
            assert_eq!(run(seed, 2_000, &common), seen);
        }
        for (sum, count) in total.iter_mut().zip(seen) {
            *sum += count;
        }
    }
    // 每一类结果都大量出现，序列没有退化成只有无效按键。
    let [pending, commands, unsupported, invalid, cancelled, unbound] = total;
    assert!(pending > 50_000, "{total:?}");
    assert!(commands > 50_000, "{total:?}");
    assert!(unsupported > 1_000, "{total:?}");
    assert!(invalid > 10_000, "{total:?}");
    assert!(cancelled > 1_000, "{total:?}");
    assert!(unbound > 10_000, "{total:?}");
}

#[test]
fn random_text_parses_as_notation_and_writes_back_stably() {
    let pieces = [
        "<", ">", "-", "C-", "S-", "M-", "D-", "A-", "Esc", "CR", "lt", "Space", "F1", "F99", "x",
        "d", "\"", " ", "\n", "\t", "中", "\u{301}", "<C-", "<S-Left>", "<Esc>", "<lt>", ">>",
    ];
    let mut rng = Rng(0x5eed);
    for round in 0..20_000 {
        let length = rng.below(12);
        let text: String = (0..length).map(|_| rng.pick(&pieces)).collect();
        let keys = notation::parse(&text);
        let written = notation::write(&keys);
        assert_eq!(
            notation::parse(&written),
            keys,
            "第 {round} 轮：{text:?} 写作 {written:?}"
        );
        assert_eq!(
            notation::write(&notation::parse(&written)),
            written,
            "第 {round} 轮"
        );
    }
}
