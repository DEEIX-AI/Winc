//! `hash` 模块的测试：FNV-1a 的已知测试值与分段写入的结果。

use super::*;

#[test]
fn matches_the_published_fnv_1a_values() {
    // 期望值取自 FNV 规范公布的 64 位 FNV-1a 测试向量；这些值一旦变化，已有的备份目录名和
    // 仓库指纹都会失效。
    assert_eq!(fnv(*b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv(*b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv(*b"foobar"), 0x8594_4171_f739_67e8);
}

/// 覆盖分段写入：`write` 与 `push` 交替调用、空片段，结果应与一次写入整段字节相同。
#[test]
fn writing_in_pieces_is_the_same_as_writing_once() {
    let mut hasher = Fnv::new();
    hasher.write(b"foo");
    hasher.push(0);
    hasher.write(b"");
    hasher.write(b"bar");
    assert_eq!(hasher.finish(), fnv(*b"foo\0bar"));
}
