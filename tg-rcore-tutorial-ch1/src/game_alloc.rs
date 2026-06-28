//! 极简全局分配器（仅 `feature = "game"`）。
//!
//! `virtio-drivers 0.1.0` 带 `extern crate alloc`，链接期要求存在一个
//! `#[global_allocator]`，即便本游戏只用到 `VirtIOGpu`（其 DMA 全部经
//! [`crate::gpu::VirtioHal`] 的静态池分配、并不走 Rust 堆）。
//!
//! 为此提供一个**小尺寸 bump 分配器**：仅为满足链接符号而存在，
//! GPU 代码路径运行时并不真正调用它。帧缓冲等大块内存仍来自 [`crate::gpu`]
//! 的固定物理地址静态 DMA 池，本章「无（动态）堆」的特性保持不变。
//! 默认判题路径（不开 `game`）不链接 virtio，也就没有此分配器，互不影响。

use core::{
    alloc::{GlobalAlloc, Layout},
    sync::atomic::{AtomicUsize, Ordering},
};

/// bump 堆字节数（保守 64 KiB；实际运行通常一字节都不会用到）。
const HEAP_SIZE: usize = 64 * 1024;

/// bump 堆后备存储（位于内核 `.bss`，QEMU 复位时 RAM 已清零）。
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// bump 游标（下一个可分配字节相对 [`HEAP`] 的偏移）。
struct BumpAlloc(AtomicUsize);

// SAFETY: 内核单核串行执行；游标用原子量推进，分配区间互不重叠。
unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = (&raw const HEAP) as usize;
        let align = layout.align();
        loop {
            let cur = self.0.load(Ordering::Relaxed);
            let start = (base + cur + align - 1) & !(align - 1);
            let next = start - base + layout.size();
            if next > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            if self
                .0
                .compare_exchange_weak(cur, next, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                return start as *mut u8;
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // bump 分配器不回收。
    }
}

/// 全局分配器实例（仅满足 `virtio-drivers` 的链接符号）。
#[global_allocator]
static GAME_HEAP: BumpAlloc = BumpAlloc(AtomicUsize::new(0));
