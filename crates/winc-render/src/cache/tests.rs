//! `Generations` 的测试：换代后刚用过的项仍然可用，按帧换代时一帧用到的项不会重做。

use super::*;

impl<K: Hash + Eq + Copy, V> Generations<K, V> {
    /// 返回两代里一共有几项。
    pub(crate) fn len(&self) -> usize {
        self.new.len() + self.old.len()
    }
}

#[test]
fn generations_keep_what_was_just_used() {
    // 一代上限为 2 字节，每项计作 1 字节：第 3 项进来后新一代超限，下一次调用开头换代；
    // 刚用过的项从旧一代搬回，不计入重新生成的次数。
    let mut cache: Generations<u32, u32> = Generations::new(2);
    let mut made = 0;
    let mut get = |cache: &mut Generations<u32, u32>, key: u32| {
        *cache.get_or_insert_with(
            key,
            |_| 1,
            || {
                made += 1;
                key * 10
            },
        )
    };
    assert_eq!(get(&mut cache, 1), 10);
    assert_eq!(get(&mut cache, 2), 20);
    assert_eq!(get(&mut cache, 3), 30);
    assert_eq!(get(&mut cache, 4), 40);
    assert_eq!(get(&mut cache, 1), 10);
    assert_eq!(get(&mut cache, 4), 40);
    assert_eq!(made, 4);
    cache.clear();
    assert_eq!(*cache.get_or_insert_with(1, |_| 1, || 11), 11);
}

#[test]
fn per_frame_generations_keep_a_frame_larger_than_one_generation() {
    // 一代只放得下 4 项，每帧按同样顺序要 8 项。帧中间就换代时，一帧要的项超过一代的容量后，
    // 每次换代都丢掉本帧还没取完的项，几乎每帧都要重新生成；只在帧首换代时，第二帧起全部命中。
    let frame = |cache: &mut Generations<u32, u32>, made: &mut usize| {
        for key in 0..8 {
            cache.get_or_insert_with(
                key,
                |_| 1,
                || {
                    *made += 1;
                    key
                },
            );
        }
    };
    let mut cache = Generations::new(4);
    let mut made = 0;
    for _ in 0..3 {
        frame(&mut cache, &mut made);
    }
    assert!(made > 8 * 2, "帧中间换代，每帧都重新生成：{made}");

    let mut cache = Generations::per_frame(4);
    let mut made = 0;
    for _ in 0..3 {
        cache.new_frame();
        frame(&mut cache, &mut made);
    }
    assert_eq!(made, 8);

    // 一帧之中也不会无限制增长：新一代的占用超过上限的 4 倍时，在调用开头换代。
    let mut cache = Generations::per_frame(1);
    let mut made = 0;
    for key in 0..10 {
        cache.get_or_insert_with(
            key,
            |_| 1,
            || {
                made += 1;
                key
            },
        );
    }
    assert!(cache.bytes <= 5);
}
