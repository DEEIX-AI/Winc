//! 载荷里各种值的读写：整数、字节串、尺寸、颜色、行、命令块和事件。

use super::*;

/// 按协议格式追写一条消息的载荷。
pub(super) struct Writer<'a> {
    /// 目标缓冲。
    out: &'a mut Vec<u8>,
    /// 长度字段的位置，[`Writer::finish`] 时回填。
    start: usize,
}

impl<'a> Writer<'a> {
    /// 先占住长度字段的位置，[`Writer::finish`] 时填上。
    pub(super) fn begin(out: &'a mut Vec<u8>) -> Self {
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        Writer { out, start }
    }

    /// 回填消息长度（小端 `u32`，不含长度字段本身）。
    pub(super) fn finish(self) {
        let len = (self.out.len() - self.start - 4) as u32;
        self.out[self.start..self.start + 4].copy_from_slice(&len.to_le_bytes());
    }

    /// 写入原样的字节。
    pub(super) fn raw(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    /// 写入一个字节。
    pub(super) fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    /// 写入一个布尔值，0 表示假，1 表示真。
    pub(super) fn bool(&mut self, v: bool) {
        self.out.push(u8::from(v));
    }

    /// 写入小端 `u16`。
    pub(super) fn u16(&mut self, v: u16) {
        self.raw(&v.to_le_bytes());
    }

    /// 写入小端 `u32`。
    pub(super) fn u32(&mut self, v: u32) {
        self.raw(&v.to_le_bytes());
    }

    /// 写入小端 `i32`。
    pub(super) fn i32(&mut self, v: i32) {
        self.raw(&v.to_le_bytes());
    }

    /// 写入小端 `u64`。
    pub(super) fn u64(&mut self, v: u64) {
        self.raw(&v.to_le_bytes());
    }

    /// 写入字节数和小端 `u32` 长度前缀。
    pub(super) fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.raw(v);
    }

    /// 写入尺寸：列数、行数、宽、高各一个小端 `u16`。
    pub(super) fn size(&mut self, size: Size) {
        self.u16(size.cols);
        self.u16(size.rows);
        self.u16(size.width);
        self.u16(size.height);
    }

    fn color(&mut self, color: Color) {
        match color {
            Color::Default => self.u8(0),
            Color::Indexed(i) => {
                self.u8(1);
                self.u8(i);
            }
            Color::Rgb(r, g, b) => {
                self.u8(2);
                self.raw(&[r, g, b]);
            }
        }
    }

    /// 写入一行：文字、样式段、软换行标记和 shell 集成标记。
    pub(super) fn line(&mut self, line: &Line) {
        self.bytes(line.text.as_bytes());
        self.u32(line.runs.len() as u32);
        for run in &line.runs {
            self.u16(run.start);
            self.color(run.style.fg);
            self.color(run.style.bg);
            self.color(run.style.underline_color);
            self.u16(run.style.attrs.0);
            self.u16(run.style.link);
        }
        self.bool(line.wrapped);
        self.u8(line.marks.flags);
        self.i32(line.marks.exit);
        self.u32(line.marks.block);
    }

    fn option_u64(&mut self, v: Option<u64>) {
        self.bool(v.is_some());
        self.u64(v.unwrap_or(0));
    }

    /// 写入一个命令块。
    pub(super) fn block(&mut self, block: &Block) {
        self.u32(block.id);
        self.u64(block.prompt);
        self.u64(block.command);
        self.option_u64(block.output);
        self.option_u64(block.end);
        self.bool(block.exit.is_some());
        self.i32(block.exit.unwrap_or(0));
        self.bool(block.duration.is_some());
        self.u32(block.duration.unwrap_or(0));
        self.bytes(block.text.as_bytes());
        self.bytes(&block.cwd);
    }

    /// 写入一个事件，前面的类型字节区分种类。
    pub(super) fn event(&mut self, event: &Event) {
        match event {
            Event::Title(title) => {
                self.u8(0);
                self.bytes(title.as_bytes());
            }
            Event::Cwd(path) => {
                self.u8(1);
                self.bytes(path);
            }
            Event::Bell => self.u8(2),
            Event::Notify { title, body } => {
                self.u8(3);
                self.bytes(title.as_bytes());
                self.bytes(body.as_bytes());
            }
            Event::Clipboard(text) => {
                self.u8(4);
                self.bytes(text.as_bytes());
            }
            Event::Link(id, uri) => {
                self.u8(5);
                self.u16(*id);
                self.bytes(uri.as_bytes());
            }
            Event::Command(mark) => {
                self.u8(6);
                match mark {
                    CommandMark::Prompt => self.u8(0),
                    CommandMark::Command => self.u8(1),
                    CommandMark::Output => self.u8(2),
                    CommandMark::Done(exit) => {
                        self.u8(3);
                        self.bool(exit.is_some());
                        self.i32(exit.unwrap_or(0));
                    }
                }
            }
        }
    }
}

/// 按协议格式读取一条消息的载荷。
pub(super) struct Reader<'a> {
    /// 收到的载荷，不含长度字段。
    data: &'a [u8],
    /// 下一个要读的字节位置（字节偏移，从 0 起）。
    pos: usize,
}

impl<'a> Reader<'a> {
    /// 从一条消息的载荷开始读。
    pub(super) fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// 读走 `n` 个字节。
    ///
    /// # Errors
    ///
    /// 剩余字节不足 `n` 个，或位置加 `n` 溢出时返回 [`DecodeError`]。
    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let bytes = self
            .data
            .get(self.pos..self.pos.checked_add(n).ok_or(DecodeError("长度溢出"))?)
            .ok_or(DecodeError("数据被截断"))?;
        self.pos += n;
        Ok(bytes)
    }

    /// 检查是否已读到载荷末尾。
    ///
    /// # Errors
    ///
    /// 还有未读的字节时返回 [`DecodeError`]。
    pub(super) fn end(&self) -> Result<(), DecodeError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(DecodeError("消息末尾有多余的字节"))
        }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        Ok(*self
            .take(N)?
            .first_chunk()
            .expect("take 返回的正好是 N 个字节"))
    }

    /// 读一个小端 `u8`。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    /// 读一个布尔值。
    ///
    /// # Errors
    ///
    /// 字节不是 0 或 1，或者数据被截断时返回 [`DecodeError`]。
    pub(super) fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError("布尔值无效")),
        }
    }

    /// 读一个小端 `u16`。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn u16(&mut self) -> Result<u16, DecodeError> {
        self.array().map(u16::from_le_bytes)
    }

    /// 读一个小端 `u32`。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn u32(&mut self) -> Result<u32, DecodeError> {
        self.array().map(u32::from_le_bytes)
    }

    /// 读一个小端 `i32`。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn i32(&mut self) -> Result<i32, DecodeError> {
        self.array().map(i32::from_le_bytes)
    }

    /// 读一个小端 `u64`。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn u64(&mut self) -> Result<u64, DecodeError> {
        self.array().map(u64::from_le_bytes)
    }

    /// 读一个 RGB 三元组。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn rgb(&mut self) -> Result<[u8; 3], DecodeError> {
        self.array()
    }

    /// 读一个元素个数。
    ///
    /// 每个元素至少占一个字节，个数超过剩余字节数时一定是坏数据，不按它分配。
    pub(super) fn count(&mut self) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > self.data.len() - self.pos {
            return Err(DecodeError("元素数量无效"));
        }
        Ok(n)
    }

    /// 先读元素个数，再按该个数逐个调用 `item` 读取；任一项出错时立即返回。
    pub(super) fn list<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let n = self.count()?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(item(self)?);
        }
        Ok(out)
    }

    /// 读一段带长度前缀的字节。
    ///
    /// # Errors
    ///
    /// 长度超出剩余字节时返回 [`DecodeError`]。
    pub(super) fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    /// 读一段 UTF-8 文字。
    ///
    /// # Errors
    ///
    /// 数据被截断或不是合法 UTF-8 时返回 [`DecodeError`]。
    pub(super) fn string(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| DecodeError("文本不是有效的 UTF-8"))
    }

    /// 读一个尺寸。
    ///
    /// # Errors
    ///
    /// 数据被截断时返回 [`DecodeError`]。
    pub(super) fn size(&mut self) -> Result<Size, DecodeError> {
        Ok(Size {
            cols: self.u16()?,
            rows: self.u16()?,
            width: self.u16()?,
            height: self.u16()?,
        })
    }

    fn color(&mut self) -> Result<Color, DecodeError> {
        match self.u8()? {
            0 => Ok(Color::Default),
            1 => Ok(Color::Indexed(self.u8()?)),
            2 => {
                let [r, g, b] = self.rgb()?;
                Ok(Color::Rgb(r, g, b))
            }
            _ => Err(DecodeError("颜色值无效")),
        }
    }

    /// 读一行。
    ///
    /// # Errors
    ///
    /// 数据被截断或字段值不合法时返回 [`DecodeError`]。
    pub(super) fn line(&mut self) -> Result<Line, DecodeError> {
        let text = self.string()?;
        let runs = self.list(|r| {
            Ok(Run {
                start: r.u16()?,
                style: Style {
                    fg: r.color()?,
                    bg: r.color()?,
                    underline_color: r.color()?,
                    attrs: Attrs(r.u16()?),
                    link: r.u16()?,
                },
            })
        })?;
        Ok(Line {
            text,
            runs,
            wrapped: self.bool()?,
            marks: Marks {
                flags: self.u8()?,
                exit: self.i32()?,
                block: self.u32()?,
            },
        })
    }

    fn option_u64(&mut self) -> Result<Option<u64>, DecodeError> {
        let has = self.bool()?;
        let v = self.u64()?;
        Ok(has.then_some(v))
    }

    /// 读一个命令块。
    ///
    /// # Errors
    ///
    /// 数据被截断或字段值不合法时返回 [`DecodeError`]。
    pub(super) fn block(&mut self) -> Result<Block, DecodeError> {
        let id = self.u32()?;
        let prompt = self.u64()?;
        let command = self.u64()?;
        let output = self.option_u64()?;
        let end = self.option_u64()?;
        let has_exit = self.bool()?;
        let exit = self.i32()?;
        let has_duration = self.bool()?;
        let duration = self.u32()?;
        Ok(Block {
            id,
            prompt,
            command,
            output,
            end,
            exit: has_exit.then_some(exit),
            duration: has_duration.then_some(duration),
            text: self.string()?,
            cwd: self.bytes()?.to_vec(),
        })
    }

    /// 读一个事件。
    ///
    /// # Errors
    ///
    /// 类型不认识或数据被截断时返回 [`DecodeError`]。
    pub(super) fn event(&mut self) -> Result<Event, DecodeError> {
        Ok(match self.u8()? {
            0 => Event::Title(self.string()?),
            1 => Event::Cwd(self.bytes()?.to_vec()),
            2 => Event::Bell,
            3 => Event::Notify {
                title: self.string()?,
                body: self.string()?,
            },
            4 => Event::Clipboard(self.string()?),
            5 => Event::Link(self.u16()?, self.string()?),
            6 => Event::Command(match self.u8()? {
                0 => CommandMark::Prompt,
                1 => CommandMark::Command,
                2 => CommandMark::Output,
                3 => {
                    let has = self.bool()?;
                    let exit = self.i32()?;
                    CommandMark::Done(has.then_some(exit))
                }
                _ => return Err(DecodeError("命令标记无效")),
            }),
            _ => return Err(DecodeError("未知的事件")),
        })
    }
}
