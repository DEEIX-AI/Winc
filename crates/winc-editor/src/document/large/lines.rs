//! 大文件文本按行读取和编辑：`Lines`、`EditBuffer` 的实现。

use super::*;

impl Lines for LargeText {
    fn line_count(&self) -> usize {
        self.total_newlines() as usize + 1
    }

    fn line_start(&self, line: usize) -> usize {
        self.locate_line(line).1 as usize
    }

    fn line_of(&self, pos: usize) -> usize {
        let pos = pos as u64;

        if pos >= self.total_chars() {
            return self.total_newlines() as usize;
        }

        let i = self.piece_at(pos);
        let (_, newlines) = self.split_point(i, pos - self.chars_before[i]);
        (self.lines_before[i] + newlines) as usize
    }

    fn line_content(&self, line: usize) -> Line<'_> {
        Line::Owned(self.with_line(line, |text, _| text.to_owned()))
    }

    fn line_len(&self, line: usize) -> usize {
        self.with_line(line, |_, chars| chars)
    }

    fn substring(&self, from: usize, to: usize) -> String {
        let (a, b) = (
            self.locate_char(from.min(to) as u64),
            self.locate_char(to as u64),
        );
        let mut bytes = Vec::new();

        self.for_each_range(a, b, |p, start, end| match p.source {
            Source::Added => {
                bytes.extend_from_slice(&self.added_bytes(p)[start as usize..end as usize])
            }
            Source::Original => {
                let old = bytes.len();
                bytes.resize(old + (end - start) as usize, 0);
                let n = self.original.read(p.start + start, &mut bytes[old..]);
                bytes.truncate(old + n);
            }
        });

        String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }

    fn end_pos(&self) -> usize {
        self.total_chars() as usize
    }

    fn visit_lines(
        &self,
        from: usize,
        max_bytes: usize,
        mut f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
    ) -> Option<usize> {
        if from >= self.line_count() {
            return None;
        }

        let cap = self.cap().min(max_bytes);
        let (loc, start) = self.locate_line(from);
        let (mut line, mut start, mut chars) = (from, start as usize, 0);
        // 跨越读取块的行先拼接到 `carry`；整行落在同一块内时直接借用，不复制。
        let (mut carry, mut scratch) = (Vec::new(), String::new());
        let mut stopped = None;

        self.stream(loc, |chunk| {
            // 每块都在完整字符处结束。整块是合法 UTF-8 时，按换行切出的每一段也合法，只需校验一次。
            let valid = std::str::from_utf8(chunk).ok();
            let count = |bytes: &[u8]| {
                if valid.is_some() {
                    count_valid(bytes)
                } else {
                    char_count(bytes)
                }
            };
            let mut at = 0;

            while let Some(i) = memchr::memchr(b'\n', &chunk[at..]).map(|i| at + i) {
                // 字符数按整行统计才能得到准确的行首位置；交给 `f` 的内容只保留前 `cap` 字节。
                chars += count(&chunk[at..i]) as usize;

                let flow = if carry.is_empty() {
                    emit_line(
                        &mut f,
                        (line, start),
                        &chunk[at..i],
                        valid.map(|t| &t[at..i]),
                        true,
                        cap,
                        &mut scratch,
                    )
                } else {
                    push_capped(&mut carry, &chunk[at..i], cap);
                    let flow =
                        emit_line(&mut f, (line, start), &carry, None, true, cap, &mut scratch);
                    carry.clear();
                    flow
                };

                if flow.is_break() {
                    stopped = Some(line);
                    return ControlFlow::Break(());
                }

                (line, start, chars) = (line + 1, start + chars + 1, 0);
                at = i + 1;
            }

            push_capped(&mut carry, &chunk[at..], cap);
            chars += count(&chunk[at..]) as usize;
            ControlFlow::Continue(())
        });

        if stopped.is_some() {
            return stopped;
        }

        // 最后一行后面没有换行符，单独交给 `f`。
        emit_line(
            &mut f,
            (line, start),
            &carry,
            None,
            false,
            cap,
            &mut scratch,
        )
        .is_break()
        .then_some(line)
    }
}

impl LargeText {
    /// 用第 `line` 行的文字和字符数调用 `f`；这一行不是最近读过的那一行时先从磁盘读出、解码并记下。
    fn with_line<R>(&self, line: usize, f: impl FnOnce(&str, usize) -> R) -> R {
        let mut last = self
            .last_line
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_, text, chars) = match &mut *last {
            Some(cached) if cached.0 == line => cached,
            slot => {
                let cap = self.cap();
                let (mut bytes, ended) = self.read_line(line, cap);
                bytes.truncate(visible_len(&bytes, ended, cap));
                // 合法的 UTF-8（绝大多数行）直接把读到的字节当作文字，不再复制一遍。
                let text = String::from_utf8(bytes)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
                let chars = text.chars().count();
                slot.insert((line, text, chars))
            }
        };
        f(text, *chars)
    }
}

impl EditBuffer for LargeText {
    fn replace(&mut self, start: usize, removed: usize, inserted: &str) {
        if removed > 0 {
            self.delete(start as u64, (start + removed) as u64);
        }

        if !inserted.is_empty() {
            self.insert(start as u64, inserted);
        }
    }

    /// 一次做完多处修改：逐处修改时每处都要在片段表中间插入、再重算其后的前缀和，几千处
    /// 的「全部替换」就是平方级。
    ///
    /// 各处按位置排好（全部替换、多光标输入从前往后，撤销从后往前）时，先换算成改动之前的
    /// 位置，再遍历一次片段表拼出新表；顺序不规则时逐处修改。
    fn replace_each(&mut self, changes: &[(usize, usize, &str)]) {
        match sorted_ranges(changes) {
            Some(ranges) if ranges.len() > 1 => self.replace_sorted(&ranges),
            _ => {
                for &(start, removed, inserted) in changes {
                    self.replace(start, removed, inserted);
                }
            }
        }
    }
}

/// 把依次修改的 `changes` 换算成改动之前的位置（起点、终点、插入的文字），按位置从前往后排好。
///
/// 各处不是从前往后或从后往前互不重叠时返回 `None`。
fn sorted_ranges<'a>(changes: &[(usize, usize, &'a str)]) -> Option<Vec<(u64, u64, &'a str)>> {
    let len = |s: &str| s.chars().count();
    let ascending = changes.windows(2).all(|w| w[1].0 >= w[0].0 + len(w[0].2));
    if ascending {
        // 前面几处改完后的位置减去它们让文字变长的部分，就是改动之前的位置。
        let mut delta = 0i64;
        let ranges = changes
            .iter()
            .map(|&(start, removed, inserted)| {
                let from = (start as i64 - delta) as u64;
                delta += len(inserted) as i64 - removed as i64;
                (from, from + removed as u64, inserted)
            })
            .collect();
        return Some(ranges);
    }
    // 从后往前：后面的修改都在前面的之前，位置不受影响。
    let descending = changes.windows(2).all(|w| w[1].0 + w[1].1 <= w[0].0);
    descending.then(|| {
        (changes.iter().rev())
            .map(|&(start, removed, inserted)| (start as u64, (start + removed) as u64, inserted))
            .collect()
    })
}
