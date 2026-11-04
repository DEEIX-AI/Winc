//! `conpty` 的测试：覆盖参数引号与 Windows 环境块的编码。

use super::*;

/// 含空格、引号和结尾反斜杠的参数按 `CommandLineToArgvW` 的规则加引号。
#[test]
fn arguments_are_quoted_like_command_line_to_argv() {
    assert_eq!(quote("plain"), "plain");
    assert_eq!(quote(""), "\"\"");
    assert_eq!(quote("a b"), "\"a b\"");
    assert_eq!(quote(r"C:\Program Files\x\"), r#""C:\Program Files\x\\""#);
    assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
    assert_eq!(quote(r#"a\"b c"#), r#""a\\\"b c""#);
}

/// 环境块按名字（不分大小写）排序，`=` 开头的名字被丢弃，结尾有两个 NUL，空表也有两个 NUL。
#[test]
fn environment_blocks_are_sorted_and_double_terminated() {
    let env = vec![
        (b"b".to_vec(), b"2".to_vec()),
        (b"A".to_vec(), b"1".to_vec()),
        (b"=C:".to_vec(), b"bad".to_vec()),
    ];
    let block = String::from_utf16(&environment_block(&env)).unwrap();
    assert_eq!(block, "A=1\0b=2\0\0");
    assert_eq!(environment_block(&[]), [0, 0]);
}
