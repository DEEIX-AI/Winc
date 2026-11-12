//! 服务进程一侧：从输入读取请求，在本机执行，把结果写到输出。
//!
//! 运行在远端主机上，由 `winc-server` 程序调用。只使用标准输入输出，不监听端口，不写任何文件；
//! 输入结束（连接断开）时返回，进程随之退出。请求中的路径是该主机上的绝对路径。

use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::wire::{self, Frame, Reader, Writer, kind};

/// 逐个处理 `input` 中的请求并把应答写到 `output`，直到输入结束。
///
/// 单个请求失败时应答错误并继续；请求不合协议（帧长度越界、帧不完整）时返回错误，调用方应
/// 结束进程。
///
/// # Errors
///
/// 读取、写入失败，或收到不合协议的帧时返回对应的 `io::Error`。
pub fn serve(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    while let Some(frame) = wire::read_frame(&mut input)? {
        match handle(&frame) {
            Ok(payload) => wire::write_frame(&mut output, kind::OK, frame.id, &payload)?,
            Err(error) => {
                let payload = wire::encode_error(&error);
                wire::write_frame(&mut output, kind::ERROR, frame.id, &payload)?;
            }
        }
        output.flush()?;
    }
    Ok(())
}

/// 执行一个请求，返回应答的载荷。
fn handle(frame: &Frame) -> io::Result<Vec<u8>> {
    let mut input = Reader::new(&frame.payload);
    match frame.kind {
        kind::HELLO => {
            let version = input.u32()?;
            input.finish()?;
            if version != wire::VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "protocol version mismatch",
                ));
            }
            Ok(Writer::default().u32(wire::VERSION).finish())
        }
        kind::METADATA | kind::SYMLINK_METADATA => {
            let path = path(&mut input)?;
            input.finish()?;
            let metadata = if frame.kind == kind::METADATA {
                crate::fs::metadata(path)?
            } else {
                crate::fs::symlink_metadata(path)?
            };
            Ok(wire::write_metadata(Writer::default(), &metadata).finish())
        }
        kind::READ_DIR => {
            let path = path(&mut input)?;
            input.finish()?;
            read_dir(path)
        }
        kind::READ => {
            let path = path(&mut input)?;
            let (offset, len) = (input.u64()?, input.u32()?);
            input.finish()?;
            if len > wire::MAX_READ {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "read too large",
                ));
            }
            let mut data = vec![0; len as usize];
            let handle = crate::fs::open(path, 0)?.into_handle();
            let read = handle.read_full(offset, &mut data)?;
            let end = read < data.len();
            Ok(Writer::default()
                .bytes(&data[..read])
                .u8(u8::from(end))
                .finish())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unknown request",
        )),
    }
}

/// 读出请求中的路径：必须是绝对路径且不含 NUL。
fn path<'a>(input: &mut Reader<'a>) -> io::Result<&'a Path> {
    let bytes = input.bytes()?;
    if bytes.first() != Some(&b'/') || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must be absolute",
        ));
    }
    Ok(Path::new(OsStr::from_bytes(bytes)))
}

/// 列出目录 `path`，编码为项数和各项的种类与名字。无法取得种类的项不列出。
fn read_dir(path: &Path) -> io::Result<Vec<u8>> {
    let mut count = 0u32;
    let mut items = Writer::default();
    for entry in crate::fs::read_dir(path)?.flatten() {
        items = items
            .u8(wire::kind_byte(entry.kind))
            .bytes(entry.name.as_bytes());
        count += 1;
    }
    let items = items.finish();
    // 整个目录在一帧中返回；项数极多、超过一帧时报错，不截断。
    if items.len() + 16 > wire::MAX_FRAME {
        return Err(io::Error::other("directory too large"));
    }
    let mut payload = Writer::default().u32(count).finish();
    payload.extend_from_slice(&items);
    Ok(payload)
}
