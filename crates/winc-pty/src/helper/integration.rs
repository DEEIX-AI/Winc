//! shell 集成的注入：不改用户的配置文件，只换几个环境变量或参数，让 shell 先读 winc 的脚本，
//! 脚本里再读取用户自己的配置。
//!
//! zsh 换 `ZDOTDIR`，bash 用 `--rcfile`，fish 把 winc 的目录加到 `XDG_DATA_DIRS`。脚本编进辅助
//! 进程，启动前内容不一致时（旧版本留下的、被改过的）重写；写不进去或不认识的 shell 不注入，只是
//! 没有命令块。
//!
//! 每个 shell 另外获得一个随机数（`WINC_NONCE`，脚本读取后即删除），报告命令行时附带该随机数。

use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// zsh 的启动脚本：`.zshenv` 把 `ZDOTDIR` 恢复成用户原来的值，`winc.zsh` 安装钩子。
const ZSH: &[(&str, &str)] = &[
    ("zsh/.zshenv", include_str!("../../shell/zshenv.zsh")),
    ("zsh/winc.zsh", include_str!("../../shell/winc.zsh")),
];
/// bash 的启动脚本，由 `--rcfile` 读入。
const BASH: &[(&str, &str)] = &[("bash/winc.bash", include_str!("../../shell/winc.bash"))];
/// fish 的启动脚本，放在 `vendor_conf.d` 里由 fish 自动读入。
const FISH: &[(&str, &str)] = &[(
    "fish/fish/vendor_conf.d/winc.fish",
    include_str!("../../shell/winc.fish"),
)];

/// 环境变量表，每项是（键，值）。
type Env = Vec<(Vec<u8>, Vec<u8>)>;

/// 按 shell 的名字（`program` 的最后一段）准备注入：写好脚本，改 `env`、`args`、`login`。
///
/// 返回脚本报告命令行时附带的随机数，没有注入时返回 `None`。只给不带参数的 shell 注入；带了
/// 参数（`-c`、脚本文件）或脚本目录 `dir` 为空时不注入。
pub fn inject(
    dir: &[u8],
    program: &[u8],
    args: &mut Vec<Vec<u8>>,
    login: &mut bool,
    env: &mut Env,
) -> Option<String> {
    if !args.is_empty() || dir.is_empty() {
        return None;
    }
    let dir = Path::new(std::ffi::OsStr::from_bytes(dir));
    let name = program.rsplit(|&b| b == b'/').next().unwrap_or(program);
    let scripts = match name {
        b"zsh" => ZSH,
        b"bash" => BASH,
        b"fish" => FISH,
        _ => return None,
    };
    if let Err(error) = write_scripts(dir, scripts) {
        eprintln!(
            "winc-pty: 无法写入 shell 集成脚本（{}）：{error}",
            dir.display()
        );
        return None;
    }
    let nonce = match nonce() {
        Ok(nonce) => nonce,
        Err(error) => {
            eprintln!("winc-pty: 无法生成 shell 集成的随机数：{error}");
            return None;
        }
    };
    set(env, b"WINC_NONCE", nonce.clone().into_bytes());
    let path = |relative: &str| dir.join(relative).as_os_str().as_bytes().to_vec();

    match name {
        b"zsh" => {
            if let Some(old) = take(env, b"ZDOTDIR") {
                set(env, b"WINC_ZDOTDIR", old);
            }
            set(env, b"ZDOTDIR", path("zsh"));
        }
        b"bash" => {
            // 登录 shell 不读 --rcfile，改成非登录启动，脚本里替它读登录 shell 的文件。
            if *login {
                *login = false;
                set(env, b"WINC_BASH_LOGIN", b"1".to_vec());
            }
            args.extend([b"--rcfile".to_vec(), path("bash/winc.bash")]);
        }
        _ => {
            let mut dirs = path("fish");
            dirs.push(b':');
            match take(env, b"XDG_DATA_DIRS") {
                Some(old) => {
                    dirs.extend_from_slice(&old);
                    set(env, b"WINC_XDG_DATA_DIRS", old);
                }
                // 没设时用 fish 的默认值；winc 的目录加在前面，默认值仍然保留。
                None => dirs.extend_from_slice(b"/usr/local/share:/usr/share"),
            }
            set(env, b"XDG_DATA_DIRS", dirs);
        }
    }
    Some(nonce)
}

/// 生成 128 位的随机数，写成 32 个十六进制数字的字符串。
fn nonce() -> io::Result<String> {
    let mut bytes = [0; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// 内容不同的脚本重写一遍。
///
/// 先写到旁边的临时文件再改名，同时启动的 shell 不会读到写了一半的脚本。
fn write_scripts(dir: &Path, scripts: &[(&str, &str)]) -> io::Result<()> {
    for (relative, content) in scripts {
        let path = dir.join(relative);
        if fs::read(&path).is_ok_and(|old| old == content.as_bytes()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&temp, content)?;
        fs::rename(&temp, &path)?;
    }
    Ok(())
}

fn take(env: &mut Env, key: &[u8]) -> Option<Vec<u8>> {
    let i = env.iter().position(|(k, _)| k == key)?;
    Some(env.remove(i).1)
}

fn set(env: &mut Env, key: &[u8], value: Vec<u8>) {
    take(env, key);
    env.push((key.to_vec(), value));
}
