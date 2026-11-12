//! 连接目标：传给 `ssh` 的主机写法。

#[cfg(test)]
mod tests;

/// 连接目标的最大长度（字节）：虚拟路径的第一段是固定前缀加连接目标，文件名一段最长 255 字节，
/// 取 200 为前缀留出余量；域名最长 253 字节，超过 200 字节的目标需要在 `~/.ssh/config` 中起别名。
const MAX_LEN: usize = 200;

/// 连接目标：`ssh` 的主机参数，可以是 `~/.ssh/config` 中的别名、主机名或 `用户@主机`。
///
/// 只接受字母、数字和 `.`、`_`、`-`、`@`，且不以 `-` 开头：目标原样作为 `ssh` 的参数和虚拟路径
/// 的一段，以 `-` 开头会被 `ssh` 当作选项，其余字符在 shell 或路径中有特殊含义。端口、跳板机和
/// IPv6 地址在 `~/.ssh/config` 中配置。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target(String);

impl Target {
    /// 校验并创建连接目标；`text` 不符合要求时返回 `None`。
    pub fn new(text: &str) -> Option<Self> {
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@');
        let valid = !text.is_empty()
            && text.len() <= MAX_LEN
            && !text.starts_with('-')
            && text.chars().all(allowed);
        valid.then(|| Self(text.to_owned()))
    }

    /// 返回连接目标的文本，可直接作为 `ssh` 的主机参数。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
