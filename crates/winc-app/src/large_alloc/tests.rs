//! 全局分配器的测试：大块的内容与清零、释放后页面立即还给系统。

use super::*;

#[test]
fn large_blocks_grow_shrink_and_keep_contents() {
    // 从小块长成大块、再缩回小块，内容一直在。
    let mut v: Vec<u8> = (0..=255).cycle().take(1000).collect();
    v.resize(3 << 20, 7);
    assert_eq!(v[999], (999 % 256) as u8);
    assert_eq!(v[(3 << 20) - 1], 7);
    v.truncate(2000);
    v.shrink_to_fit();
    assert_eq!(v[999], (999 % 256) as u8);
    assert_eq!(v.len(), 2000);

    // 大块里清零的分配是零。
    let zeros = vec![0u32; LARGE];
    assert!(zeros.iter().all(|&x| x == 0));
}

/// `[ptr, ptr + len)` 里有几页在物理内存里（`mincore` 的 `MINCORE_INCORE`）。
fn resident_pages(ptr: *mut u8, len: usize) -> usize {
    let mut pages = vec![0 as libc::c_char; len.div_ceil(4096)];
    // SAFETY: `pages` 每页一个字节，足够大；`mincore` 只读地址范围的状态，不访问其中的内容。
    unsafe { libc::mincore(ptr.cast(), len, pages.as_mut_ptr()) };
    pages
        .iter()
        .filter(|&&p| p as i32 & libc::MINCORE_INCORE != 0)
        .count()
}

#[test]
fn large_blocks_go_back_to_the_system_when_freed() {
    // 一张卡片快照大小的块：释放后页面马上还给系统，不留在分配器的缓存里算进进程内存。
    let layout = Layout::from_size_align(2_831_155, 8).unwrap();
    // 释放以后这段地址可能马上被并行运行的其他测试用于分配大块并写入，看起来又在内存里了；
    // 因此重试多次，只要有一次全部归还即通过。没有归还时每次都在内存里，仍然查得出来。
    let released = (0..10).any(|_| {
        // SAFETY: `layout` 大小不为 0；写入不超出分配的范围，释放时用同一个 `layout`。
        unsafe {
            let ptr = LargeAlloc.alloc(layout);
            assert!(!ptr.is_null());
            ptr.write_bytes(1, layout.size());
            assert!(resident_pages(ptr, layout.size()) > 0);
            LargeAlloc.dealloc(ptr, layout);
            resident_pages(ptr, layout.size()) == 0
        }
    });
    assert!(released, "释放以后页面还在内存里");
}
