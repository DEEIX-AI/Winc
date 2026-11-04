//! 后台任务的代号：同一件事（同一个键，例如同一个文档）可能先后发出多次，只收最后一次的结果。
//!
//! 先发出的不一定先做完：先发的结果晚到时，按版本号判断会把后发、更新的结果当成过时的结果丢弃，
//! 或者反过来用旧结果覆盖新结果。发出时领一个代号，收结果时核对代号是否仍是该键最新的一个。

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

/// 各键最近一次发出的任务的代号。
pub(crate) struct Tickets<K> {
    /// 下一个代号；在所有键之间递增，不复用。
    next: u64,
    /// 各键在等的那一次。
    latest: HashMap<K, u64>,
}

impl<K> Default for Tickets<K> {
    fn default() -> Self {
        Self {
            next: 0,
            latest: HashMap::new(),
        }
    }
}

impl<K: Hash + Eq> Tickets<K> {
    /// 为 `key` 发出一次任务，返回它的代号；之前发出、还没收的那次随之作废。
    pub(crate) fn issue(&mut self, key: K) -> u64 {
        self.next += 1;
        self.latest.insert(key, self.next);
        self.next
    }

    /// `key` 是否有发出了、还没收的那次。
    pub(crate) fn is_waiting<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.latest.contains_key(key)
    }

    /// 收下 `key` 的一次结果：`ticket` 是该键最近一次的代号时返回 `true`，不再等该次任务；已作废
    /// （之后又发出过，或者已经收过）时返回 `false`。
    pub(crate) fn redeem<Q>(&mut self, key: &Q, ticket: u64) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if self.latest.get(key) != Some(&ticket) {
            return false;
        }
        self.latest.remove(key);
        true
    }
}

#[cfg(test)]
mod tests;
