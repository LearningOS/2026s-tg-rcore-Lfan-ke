//! VirtIO-GPU 帧缓冲驱动（第四章图形游戏基建）。
//!
//! 复用第八章 `virtio_block.rs` 的 HAL 思路：内核对内核镜像与堆做恒等映射，
//! 因此 DMA 分配（来自内核堆）的物理地址与内核虚址相同，HAL 的
//! `phys_to_virt` / `virt_to_phys` 都退化为恒等变换。GPU 与键盘
//! （见 `input.rs`）共用本 HAL。
//!
//! 帧缓冲像素格式 `BGRA8888`（内存序 `[B, G, R, A]`），分辨率由设备上报
//! （运行器固定 640×480）。`setup_framebuffer` 返回的切片即帧缓冲 DMA 区，
//! 其物理地址（= 内核恒等虚址）会被映射进每个用户进程的 `0x4000_0000`。

use crate::Sv39;
use alloc::alloc::{alloc_zeroed, dealloc};
use core::{
    alloc::Layout,
    ptr::NonNull,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};
use spin::{Lazy, Mutex};
use tg_kernel_vm::page_table::MmuMeta;
use virtio_drivers::{Hal, MmioTransport, VirtIOGpu, VirtIOHeader};

/// GPU 设备 MMIO 槽位（ch4 无块设备 → `virtio-mmio-bus.0`）。
const VIRTIO_GPU: usize = 0x1000_1000;

/// VirtIO HAL：内核恒等映射下物理地址 == 内核虚址。
///
/// 对 GPU 与键盘均适用，故在两个驱动间共享。
pub struct VirtioHal;

impl Hal for VirtioHal {
    /// 分配 `pages` 个连续物理页（来自内核堆，已清零）。
    fn dma_alloc(pages: usize) -> usize {
        unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                pages << Sv39::PAGE_BITS,
                1 << Sv39::PAGE_BITS,
            )) as usize
        }
    }
    /// 释放 DMA 物理页。
    fn dma_dealloc(paddr: usize, pages: usize) -> i32 {
        unsafe {
            dealloc(
                paddr as *mut u8,
                Layout::from_size_align_unchecked(pages << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS),
            );
        }
        0
    }
    /// 物理地址 → 虚拟地址（恒等）。
    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }
    /// 虚拟地址 → 物理地址（内核恒等映射）。
    fn virt_to_phys(vaddr: usize) -> usize {
        vaddr
    }
}

/// 帧缓冲物理地址（= 内核恒等虚址，页对齐）。
static FB_PADDR: AtomicUsize = AtomicUsize::new(0);
/// 帧缓冲字节长度。
static FB_LEN: AtomicUsize = AtomicUsize::new(0);
/// 屏幕宽度（像素）。
static FB_WIDTH: AtomicU32 = AtomicU32::new(0);
/// 屏幕高度（像素）。
static FB_HEIGHT: AtomicU32 = AtomicU32::new(0);

/// VirtIO-GPU 设备封装。
struct Gpu(VirtIOGpu<'static, VirtioHal, MmioTransport>);

// SAFETY: 全局实例由 Mutex 串行化访问，内部裸指针不跨线程共享。
unsafe impl Send for Gpu {}

/// 全局 GPU 实例（延迟初始化）。
static GPU: Lazy<Mutex<Gpu>> = Lazy::new(|| {
    let transport = unsafe {
        MmioTransport::new(NonNull::new(VIRTIO_GPU as *mut VirtIOHeader).unwrap())
            .expect("virtio-gpu: invalid MMIO transport")
    };
    let mut gpu = VirtIOGpu::<VirtioHal, _>::new(transport).expect("virtio-gpu: init failed");
    let (width, height) = gpu.resolution().expect("virtio-gpu: query resolution failed");
    let fb = gpu
        .setup_framebuffer()
        .expect("virtio-gpu: setup framebuffer failed");
    // fb 是 'static DMA 切片：恒等映射下 as_ptr() 即物理地址。
    FB_PADDR.store(fb.as_ptr() as usize, Ordering::SeqCst);
    FB_LEN.store(fb.len(), Ordering::SeqCst);
    FB_WIDTH.store(width, Ordering::SeqCst);
    FB_HEIGHT.store(height, Ordering::SeqCst);
    Mutex::new(Gpu(gpu))
});

/// 初始化 GPU 与键盘。
///
/// 必须在内核地址空间建立、MMIO 区域映射完成之后调用（否则访问设备寄存器
/// 触发缺页）。初始化后帧缓冲信息可经 [`fb_paddr`] / [`fb_len`] 等查询。
pub fn init() {
    Lazy::force(&GPU);
    crate::input::init();
}

/// 帧缓冲物理地址（页对齐）。
pub fn fb_paddr() -> usize {
    FB_PADDR.load(Ordering::SeqCst)
}

/// 帧缓冲字节长度（`width * height * 4`）。
pub fn fb_len() -> usize {
    FB_LEN.load(Ordering::SeqCst)
}

/// 屏幕宽度（像素）。
pub fn fb_width() -> u32 {
    FB_WIDTH.load(Ordering::SeqCst)
}

/// 屏幕高度（像素）。
pub fn fb_height() -> u32 {
    FB_HEIGHT.load(Ordering::SeqCst)
}

/// 把帧缓冲内容推送到屏幕。
pub fn flush() {
    GPU.lock().0.flush().expect("virtio-gpu: flush failed");
}
