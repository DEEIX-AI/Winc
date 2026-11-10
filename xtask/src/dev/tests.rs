//! 影响范围计算的测试：改动的文件如何带出依赖这些包的包，以及全局文件的影响。

use super::*;

fn member(name: &str, dir: &str, deps: &[&str]) -> Member {
    Member {
        name: name.to_owned(),
        dir: PathBuf::from(dir),
        deps: deps.iter().map(|d| (*d).to_owned()).collect(),
    }
}

#[test]
fn changes_pull_in_the_crates_that_depend_on_them() {
    // 依赖链为 `winc-core` ← `winc-editor` ← {`winc-app`, `xtask`}；`winc-render` 不依赖任何包。
    let members = [
        member("winc-app", "crates/winc-app", &["winc-editor", "winc-core"]),
        member("winc-core", "crates/winc-core", &[]),
        member("winc-editor", "crates/winc-editor", &["winc-core"]),
        member("winc-render", "crates/winc-render", &[]),
        member("xtask", "xtask", &["winc-editor"]),
    ];
    let affected = |files: &[&str]| {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        affected(&members, &files)
    };
    assert_eq!(
        affected(&["crates/winc-render/src/lib.rs"]),
        ["winc-render"]
    );
    assert_eq!(affected(&["crates/winc-app/src/app.rs"]), ["winc-app"]);
    assert_eq!(
        affected(&["crates/winc-core/src/text.rs"]),
        ["winc-app", "winc-core", "winc-editor", "xtask"]
    );
    assert!(affected(&["docs/GIT.md"]).is_empty());
    // 根目录的 `Cargo.lock`、`VERSION` 变了时全部 5 个包都要测。
    assert_eq!(affected(&["Cargo.lock"]).len(), 5);
    assert_eq!(affected(&["VERSION"]).len(), 5);
}
