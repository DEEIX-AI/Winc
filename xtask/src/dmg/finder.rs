//! dmg 打开后的窗口布局：窗口大小、背景图、图标大小和位置，都写在卷根目录的 `.DS_Store` 里。
//!
//! 只实现用得到的部分：`.DS_Store` 由伙伴分配器（buddy allocator）分成块，一棵 B 树存各文件的记录
//! （文件名、四个字符的记录种类、值），窗口和图标视图的设置是二进制属性列表（`bplist00`），作为
//! 记录的值存进去；背景图用第 2 版别名记录指向卷里的图片，系统按卷名、卷创建时间和文件编号
//! （HFS+ 的 CNID）定位图片。

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// `.DS_Store` 中的一条记录。
pub struct Record {
    /// 记录所属的文件名；`.` 表示目录本身，也就是卷的根目录。
    pub name: String,
    /// 记录种类，例如 `Iloc`（图标位置）、`bwsp`（窗口）、`icvp`（图标视图）。
    pub code: [u8; 4],
    /// 记录的值。
    pub value: Value,
}

/// 记录的值。
pub enum Value {
    /// 32 位整数（`long`）。
    Long(u32),
    /// 四个字符的类型码（`type`）。
    Type([u8; 4]),
    /// 一段字节（`blob`）。
    Blob(Vec<u8>),
}

impl Record {
    /// 返回文件 `name` 的图标位置记录，图标中心为 `(x, y)`（窗口内容区坐标，点）。
    pub fn icon_at(name: &str, (x, y): (u32, u32)) -> Record {
        let mut location = Vec::with_capacity(16);
        location.extend(x.to_be_bytes());
        location.extend(y.to_be_bytes());
        // 后 8 个字节是固定的占位值，系统写入的记录也使用这两个值。
        location.extend(0xFFFF_FFFF_u32.to_be_bytes());
        location.extend(0xFFFF_0000_u32.to_be_bytes());
        Record {
            name: name.to_owned(),
            code: *b"Iloc",
            value: Value::Blob(location),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        let name: Vec<u16> = self.name.encode_utf16().collect();
        out.extend((name.len() as u32).to_be_bytes());
        for unit in name {
            out.extend(unit.to_be_bytes());
        }
        out.extend(self.code);
        match &self.value {
            Value::Long(n) => {
                out.extend(b"long");
                out.extend(n.to_be_bytes());
            }
            Value::Type(code) => {
                out.extend(b"type");
                out.extend(code);
            }
            Value::Blob(bytes) => {
                out.extend(b"blob");
                out.extend((bytes.len() as u32).to_be_bytes());
                out.extend(bytes);
            }
        }
    }
}

/// 分配器里的一块：偏移与块大小以 2 为底的对数。
///
/// 偏移从文件第 4 个字节算起。
type Block = (u32, u32);

/// 文件头占用的块：偏移 0、大小 32 字节，不在块表里，但不能当作空闲。
const HEADER: Block = (0, 5);
/// B 树描述占用的块：偏移紧接文件头的 32 字节，大小同为 32 字节，内容 20 字节。
const TREE: Block = (32, 5);
/// 分配器信息占用的块：偏移取 2 的 11 次幂（2048 字节），容纳块表、目录与空闲表；
/// 与前面的块之间的空隙由空闲表记录。
const INFO: Block = (2048, 11);
/// 存放全部记录的叶子节点占用的块：偏移取 2 的 12 次幂（4096 字节），
/// 记录总长超过该容量时返回错误。
const NODE: Block = (4096, 12);

/// 把 `records` 写成 `.DS_Store` 文件的内容。
///
/// 记录按文件名（不分大小写）、再按记录种类排序。
///
/// # Errors
///
/// 记录太多、一个节点放不下时返回原因。
pub fn ds_store(mut records: Vec<Record>) -> Result<Vec<u8>, String> {
    records.sort_by_key(|r| (r.name.to_lowercase(), r.code));
    let mut node = Vec::new();
    node.extend(0u32.to_be_bytes()); // 0：叶子节点，没有子节点。
    node.extend((records.len() as u32).to_be_bytes());
    for record in &records {
        record.write(&mut node);
    }
    if node.len() > 1 << NODE.1 {
        return Err(format!(
            ".DS_Store 的记录有 {} 字节，一个节点放不下",
            node.len()
        ));
    }

    // 块表：0 号是分配器信息自己，1 号是 B 树描述，2 号是节点。
    let address = |(offset, log2): Block| offset | log2;
    let blocks = [INFO, TREE, NODE];
    let mut info = Vec::new();
    info.extend((blocks.len() as u32).to_be_bytes());
    info.extend(0u32.to_be_bytes());
    // 块表按 256 项一组补齐。
    for i in 0..256 {
        info.extend(blocks.get(i).map_or(0, |&b| address(b)).to_be_bytes());
    }
    // 目录只有一项：B 树「DSDB」在 1 号块。
    info.extend(1u32.to_be_bytes());
    info.push(4);
    info.extend(b"DSDB");
    info.extend(1u32.to_be_bytes());
    // 空闲表：32 种大小各一张，列出没被占用的块的偏移。
    let mut free = vec![Vec::new(); 32];
    free_blocks((0, 31), &[HEADER, TREE, INFO, NODE], &mut free);
    for list in &free {
        info.extend((list.len() as u32).to_be_bytes());
        for offset in list {
            info.extend(offset.to_be_bytes());
        }
    }

    let mut tree = Vec::new();
    tree.extend(2u32.to_be_bytes()); // 根节点：2 号块。
    tree.extend(0u32.to_be_bytes()); // 根节点之上没有内部节点。
    tree.extend((records.len() as u32).to_be_bytes());
    tree.extend(1u32.to_be_bytes()); // 节点数。
    tree.extend(0x1000u32.to_be_bytes()); // 节点的页大小。

    let end = NODE.0 + (1 << NODE.1);
    let mut file = vec![0u8; 4 + end as usize];
    let mut put = |(offset, _): Block, bytes: &[u8]| {
        file[4 + offset as usize..][..bytes.len()].copy_from_slice(bytes);
    };
    put(TREE, &tree);
    put(INFO, &info);
    put(NODE, &node);
    file[..4].copy_from_slice(&1u32.to_be_bytes());
    file[4..8].copy_from_slice(b"Bud1");
    file[8..12].copy_from_slice(&INFO.0.to_be_bytes());
    file[12..16].copy_from_slice(&(1u32 << INFO.1).to_be_bytes());
    file[16..20].copy_from_slice(&INFO.0.to_be_bytes());
    Ok(file)
}

/// 把块 `block` 中没被 `used` 占用的部分按伙伴分配的方式拆开，记进 `free`（按大小的对数分表）。
fn free_blocks(block: Block, used: &[Block], free: &mut [Vec<u32>]) {
    let (offset, log2) = block;
    let end = u64::from(offset) + (1u64 << log2);
    if used.contains(&block) {
        return;
    }
    if !used
        .iter()
        .any(|&(o, _)| u64::from(o) >= u64::from(offset) && u64::from(o) < end)
    {
        free[log2 as usize].push(offset);
        return;
    }
    let half = 1u32 << (log2 - 1);
    free_blocks((offset, log2 - 1), used, free);
    free_blocks((offset + half, log2 - 1), used, free);
}

/// 属性列表中的值。
pub enum Plist {
    /// 布尔值。
    Bool(bool),
    /// 32 位整数。
    Int(u32),
    /// 64 位浮点数。
    Real(f64),
    /// 只用到 ASCII 字符串。
    Str(String),
    /// 一段字节（`data`）。
    Data(Vec<u8>),
}

/// 把只有一层的字典 `dict` 写成二进制属性列表（`bplist00`）。
///
/// 对象依次是字典、各个键、各个值，引用用一个字节，所以最多 127 项。
///
/// # Panics
///
/// `dict` 超过 127 项，或某个键、字符串不是 ASCII 时 panic。
pub fn bplist(dict: &[(&str, Plist)]) -> Vec<u8> {
    let n = dict.len();
    assert!(1 + 2 * n < 256, "属性列表的项太多");
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::with_capacity(1 + 2 * n);

    offsets.push(out.len());
    marker(&mut out, 0xD, n);
    out.extend((1..=2 * n).map(|i| i as u8));
    for (key, _) in dict {
        offsets.push(out.len());
        assert!(key.is_ascii());
        marker(&mut out, 0x5, key.len());
        out.extend(key.as_bytes());
    }
    for (_, value) in dict {
        offsets.push(out.len());
        match value {
            Plist::Bool(b) => out.push(if *b { 0x09 } else { 0x08 }),
            Plist::Int(i) => int(&mut out, *i),
            Plist::Real(r) => {
                out.push(0x23);
                out.extend(r.to_be_bytes());
            }
            Plist::Str(s) => {
                assert!(s.is_ascii());
                marker(&mut out, 0x5, s.len());
                out.extend(s.as_bytes());
            }
            Plist::Data(d) => {
                marker(&mut out, 0x4, d.len());
                out.extend(d);
            }
        }
    }

    let table = out.len();
    let width: usize = if table < 1 << 8 {
        1
    } else if table < 1 << 16 {
        2
    } else {
        4
    };
    for offset in &offsets {
        out.extend(&(*offset as u64).to_be_bytes()[8 - width..]);
    }
    out.extend([0; 6]);
    out.push(width as u8);
    out.push(1); // 对象引用的字节数。
    out.extend((offsets.len() as u64).to_be_bytes());
    out.extend(0u64.to_be_bytes()); // 顶层对象：字典。
    out.extend((table as u64).to_be_bytes());
    out
}

/// 写对象的类型标记和长度：长度不到 15 时放在标记的低 4 位，否则跟一个整数对象。
fn marker(out: &mut Vec<u8>, kind: u8, len: usize) {
    if len < 15 {
        out.push(kind << 4 | len as u8);
    } else {
        out.push(kind << 4 | 0xF);
        int(out, len as u32);
    }
}

/// 写一个整数对象，用够用的最少字节（1、2、4 字节都是无符号的）。
fn int(out: &mut Vec<u8>, n: u32) {
    if n < 1 << 8 {
        out.push(0x10);
        out.push(n as u8);
    } else if n < 1 << 16 {
        out.push(0x11);
        out.extend((n as u16).to_be_bytes());
    } else {
        out.push(0x12);
        out.extend(n.to_be_bytes());
    }
}

/// 1904-01-01 到 1970-01-01 的秒数（秒）。
///
/// 经典 Mac 的 HFS 时间从 1904 年算起，换算到 Unix 时间需要加上这段差值：66 年含 17 个闰日，
/// 共 24 107 天，即 2 082 844 800 秒。
const MAC_EPOCH: f64 = 2_082_844_800.0;

/// 别名记录（第 2 版，`alis`）附加信息的标签号，取值由该格式规定。
const TAG_FOLDER_NAME: u16 = 0;
const TAG_CNID_PATH: u16 = 1;
const TAG_CARBON_PATH: u16 = 2;
const TAG_UNICODE_FILENAME: u16 = 14;
const TAG_UNICODE_VOLUME_NAME: u16 = 15;
const TAG_VOLUME_DATE: u16 = 16;
const TAG_CREATION_DATE: u16 = 17;
const TAG_POSIX_PATH: u16 = 18;
const TAG_MOUNT_POINT: u16 = 19;

/// 返回指向 `target` 的别名记录。
///
/// `volume` 是挂载着的卷的根目录（`target` 在卷根目录下），卷名是 `name`；`mount_point` 是用户
/// 打开 dmg 时卷通常挂载的位置（`/Volumes/` 加上卷名）。
///
/// # Errors
///
/// 读不到文件或卷的信息、`target` 不在 `volume` 下面时返回错误。
pub fn alias(volume: &Path, name: &str, mount_point: &str, target: &Path) -> io::Result<Vec<u8>> {
    let relative = target
        .strip_prefix(volume)
        .map_err(|_| io::Error::other("背景图不在卷里"))?;
    let parent = target.parent().unwrap_or(volume);
    let file_name = target
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let folder_name = parent
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let volume_date = mac_time(std::fs::metadata(volume)?.created()?);
    let meta = std::fs::metadata(target)?;
    let date = mac_time(meta.created()?);
    // 从里往外各级目录的编号，不含卷的根目录。
    let mut cnid_path = Vec::new();
    for dir in target.ancestors().skip(1) {
        if dir == volume {
            break;
        }
        cnid_path.push(std::fs::metadata(dir)?.ino() as u32);
    }
    let parts: Vec<String> = relative
        .iter()
        .map(|p| p.to_string_lossy().replace(':', "/"))
        .collect();

    let mut out = Vec::new();
    out.extend([0; 4]); // 创建者的应用信息。
    out.extend([0; 2]); // 记录的总长度，最后再填。
    out.extend(2u16.to_be_bytes()); // 版本。
    out.extend(0u16.to_be_bytes()); // 目标是文件。
    pascal(&mut out, &name.replace(':', "/"), 28);
    out.extend((volume_date as u32).to_be_bytes());
    out.extend(b"H+");
    out.extend(5u16.to_be_bytes()); // 可推出的磁盘：磁盘镜像就是这样挂载的。
    out.extend((std::fs::metadata(parent)?.ino() as u32).to_be_bytes());
    pascal(&mut out, &file_name.replace(':', "/"), 64);
    out.extend((meta.ino() as u32).to_be_bytes());
    out.extend((date as u32).to_be_bytes());
    out.extend([0; 8]); // 创建者和文件类型。
    out.extend((-1i16).to_be_bytes()); // 往上找几级、往下找几级：不限。
    out.extend((-1i16).to_be_bytes());
    out.extend([0; 4 + 2 + 10]); // 卷的属性、文件系统编号、保留。

    tag(
        &mut out,
        TAG_FOLDER_NAME,
        folder_name.replace(':', "/").as_bytes(),
    );
    tag(
        &mut out,
        TAG_VOLUME_DATE,
        &((volume_date * 65536.0) as u64).to_be_bytes(),
    );
    tag(
        &mut out,
        TAG_CREATION_DATE,
        &((date * 65536.0) as u64).to_be_bytes(),
    );
    let cnids: Vec<u8> = cnid_path.iter().flat_map(|c| c.to_be_bytes()).collect();
    tag(&mut out, TAG_CNID_PATH, &cnids);
    let carbon = format!("{}:{}", name.replace(':', "/"), parts.join(":"));
    tag(&mut out, TAG_CARBON_PATH, carbon.as_bytes());
    tag(&mut out, TAG_UNICODE_FILENAME, &unicode(&file_name));
    tag(&mut out, TAG_UNICODE_VOLUME_NAME, &unicode(name));
    let posix = format!("/{}", relative.to_string_lossy());
    tag(&mut out, TAG_POSIX_PATH, posix.as_bytes());
    tag(&mut out, TAG_MOUNT_POINT, mount_point.as_bytes());
    out.extend((-1i16).to_be_bytes()); // 结束。
    out.extend(0u16.to_be_bytes());

    let len = out.len() as u16;
    out[4..6].copy_from_slice(&len.to_be_bytes());
    Ok(out)
}

/// 把时间 `time` 换算成从 1904 年起的秒数。
fn mac_time(time: SystemTime) -> f64 {
    let unix = time
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    unix + MAC_EPOCH
}

/// 写一个占 `field` 字节的 Pascal 字符串：一个字节的长度、内容，不足补零。
fn pascal(out: &mut Vec<u8>, text: &str, field: usize) {
    let mut bytes = text.as_bytes();
    if bytes.len() > field - 1 {
        // 截断时退到完整的 UTF-8 字符边界。
        let mut end = field - 1;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        bytes = &bytes[..end];
    }
    out.push(bytes.len() as u8);
    out.extend(bytes);
    out.extend(std::iter::repeat_n(0, field - 1 - bytes.len()));
}

/// 写一项附加信息：标签、长度、内容；长度为奇数时补一个零字节。
fn tag(out: &mut Vec<u8>, tag: u16, bytes: &[u8]) {
    out.extend(tag.to_be_bytes());
    out.extend((bytes.len() as u16).to_be_bytes());
    out.extend(bytes);
    if bytes.len() % 2 == 1 {
        out.push(0);
    }
}

/// 写一个 UTF-16 字符串：字符数，后跟大端编码的 UTF-16。
fn unicode(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.replace(':', "/").encode_utf16().collect();
    let mut out = Vec::with_capacity(2 + units.len() * 2);
    out.extend((units.len() as u16).to_be_bytes());
    for unit in units {
        out.extend(unit.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests;
