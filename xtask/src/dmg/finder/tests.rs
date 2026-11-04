//! `.DS_Store` 与二进制属性列表的测试：写出的结构能按格式读回，类型和内容都对。

use super::*;

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// 按格式读回写出的 `.DS_Store`：文件头指向分配器信息，目录指向 B 树，B 树指向节点，
/// 节点里是排好序的记录；空闲块和占用的块不重叠，合起来正好铺满整个空间。
#[test]
fn ds_store_reads_back() {
    let records = vec![
        Record::icon_at("winc.app", (160, 176)),
        Record {
            name: ".".into(),
            code: *b"vSrn",
            value: Value::Long(1),
        },
        Record::icon_at("Applications", (480, 176)),
        Record {
            name: ".".into(),
            code: *b"icvl",
            value: Value::Type(*b"icnv"),
        },
    ];
    let file = ds_store(records).unwrap();
    assert_eq!(&file[..8], b"\0\0\0\x01Bud1");
    let block = |address: u32| 4 + (address & !0x1F) as usize;

    let info = block(u32_at(&file, 8));
    assert_eq!(u32_at(&file, 8), u32_at(&file, 16));
    let count = u32_at(&file, info) as usize;
    let addresses: Vec<u32> = (0..count)
        .map(|i| u32_at(&file, info + 8 + i * 4))
        .collect();
    let toc = info + 8 + 256 * 4;
    assert_eq!(u32_at(&file, toc), 1);
    assert_eq!(&file[toc + 4..toc + 9], b"\x04DSDB");
    let tree = block(addresses[u32_at(&file, toc + 9) as usize]);
    let node = block(addresses[u32_at(&file, tree) as usize]);
    assert_eq!(u32_at(&file, tree + 8), 4, "记录数");
    assert_eq!(u32_at(&file, node), 0, "叶子节点");
    assert_eq!(u32_at(&file, node + 4), 4);

    // 按排序后的顺序检查记录：目录本身的记录在前，文件名不分大小写比较。
    let mut at = node + 8;
    let mut seen = Vec::new();
    for _ in 0..4 {
        let len = u32_at(&file, at) as usize;
        let units: Vec<u16> = (0..len)
            .map(|i| u16::from_be_bytes([file[at + 4 + i * 2], file[at + 5 + i * 2]]))
            .collect();
        at += 4 + len * 2;
        let code = String::from_utf8(file[at..at + 4].to_vec()).unwrap();
        let kind = &file[at + 4..at + 8];
        at += 8;
        at += match kind {
            b"long" | b"type" => 4,
            b"blob" => 4 + u32_at(&file, at) as usize,
            _ => panic!("未知类型"),
        };
        seen.push((String::from_utf16(&units).unwrap(), code));
    }
    let names: Vec<(&str, &str)> = seen.iter().map(|(n, c)| (n.as_str(), c.as_str())).collect();
    assert_eq!(
        names,
        [
            (".", "icvl"),
            (".", "vSrn"),
            ("Applications", "Iloc"),
            ("winc.app", "Iloc")
        ]
    );

    // 空闲表和占用的块（含文件头）合起来正好 2^31 字节，互不重叠。
    let mut free_at = toc + 13;
    let mut spans: Vec<(u64, u64)> = [HEADER, TREE, INFO, NODE]
        .iter()
        .map(|&(o, l)| (u64::from(o), 1u64 << l))
        .collect();
    for log2 in 0..32 {
        let n = u32_at(&file, free_at) as usize;
        for i in 0..n {
            spans.push((u64::from(u32_at(&file, free_at + 4 + i * 4)), 1u64 << log2));
        }
        free_at += 4 + n * 4;
    }
    spans.sort_unstable();
    let mut end = 0;
    for (offset, size) in spans {
        assert_eq!(offset, end, "块之间有空隙或重叠");
        end = offset + size;
    }
    assert_eq!(end, 1 << 31);
}

/// 写出的属性列表能由系统的 `plutil` 解析：覆盖布尔、整数、浮点、短字符串、长字符串和字节串。
#[test]
#[cfg(target_os = "macos")]
fn bplist_is_understood_by_plutil() {
    let long = "x".repeat(20);
    let bytes = bplist(&[
        ("ShowToolbar", Plist::Bool(false)),
        ("SidebarWidth", Plist::Int(300)),
        ("iconSize", Plist::Real(128.0)),
        (
            "WindowBounds",
            Plist::Str("{{200, 120}, {640, 400}}".into()),
        ),
        ("long", Plist::Str(long.clone())),
        ("data", Plist::Data(vec![1, 2, 3])),
    ]);
    let dir = std::env::temp_dir().join(format!("xtask-bplist-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.plist");
    std::fs::write(&path, bytes).unwrap();
    let output = std::process::Command::new("plutil")
        .args(["-convert", "xml1", "-o", "-"])
        .arg(&path)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let xml = String::from_utf8(output.stdout).unwrap();
    for part in [
        "<key>ShowToolbar</key>\n\t<false/>",
        "<key>SidebarWidth</key>\n\t<integer>300</integer>",
        "<key>iconSize</key>\n\t<real>128</real>",
        "<string>{{200, 120}, {640, 400}}</string>",
        &format!("<string>{long}</string>"),
        "<data>\n\tAQID\n\t</data>",
    ] {
        assert!(xml.contains(part), "缺少 {part}：\n{xml}");
    }
}

#[test]
fn pascal_strings_fill_their_field() {
    let mut out = Vec::new();
    pascal(&mut out, "winc", 28);
    assert_eq!(out.len(), 28);
    assert_eq!(&out[..5], b"\x04winc");
    let mut out = Vec::new();
    pascal(&mut out, &"文".repeat(30), 28);
    assert_eq!(out.len(), 28);
    assert_eq!(out[0], 27, "截在完整的字符上：9 个三字节的字");
}
