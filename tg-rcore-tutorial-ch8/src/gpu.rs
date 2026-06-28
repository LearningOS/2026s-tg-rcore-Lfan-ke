//! VirtIO-GPU 帧缓冲驱动（第八章图形游戏基建）。
//!
//! 与第七章 `gpu.rs` 完全一致（块设备占 `virtio-mmio-bus.0`，GPU 落在
//! `bus.1`=`0x1000_2000`，键盘见 `input.rs` 落在 `bus.2`=`0x1000_3000`）：
//! 1. **MMIO 槽位**：`VIRTIO_GPU = 0x1000_2000`。
//! 2. **HAL 地址翻译**：第八章同样有全局 `KERNEL_SPACE`，`virt_to_phys` 复用
//!    `virtio_block.rs` 的做法——经 `KERNEL_SPACE.translate` 查页表。GPU 与键盘
//!    （`input.rs`）共享本 HAL。
//!
//! 帧缓冲像素格式 `BGRA8888`（内存序 `[B, G, R, A]`），分辨率由设备上报。
//! `setup_framebuffer` 返回的切片即帧缓冲 DMA 区，其物理地址被映射进 DOOM
//! 进程的用户空间 `0x4000_0000`（见 `Process::map_framebuffer`）：进程内的
//! 渲染线程直接写像素、逻辑线程读输入，二者共享同一地址空间。

use crate::{build_flags, Sv39, KERNEL_SPACE};
use alloc::alloc::{alloc_zeroed, dealloc};
use core::{
    alloc::Layout,
    ptr::NonNull,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};
use spin::{Lazy, Mutex};
use tg_kernel_vm::page_table::{MmuMeta, VAddr, VmFlags};
use virtio_drivers::{Hal, MmioTransport, VirtIOGpu, VirtIOHeader};

/// GPU 设备 MMIO 槽位（第七章块设备占 bus.0 → GPU 用 `virtio-mmio-bus.1`）。
const VIRTIO_GPU: usize = 0x1000_2000;

/// VirtIO HAL：DMA 缓冲来自内核堆，地址翻译走 `KERNEL_SPACE` 页表。
///
/// 与第七章 `virtio_block.rs` 的 HAL 同构，对 GPU 与键盘均适用，故共享。
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
    /// 物理地址 → 虚拟地址（内核恒等映射下相等）。
    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }
    /// 虚拟地址 → 物理地址（经 `KERNEL_SPACE` 页表翻译，与块设备一致）。
    fn virt_to_phys(vaddr: usize) -> usize {
        const VALID: VmFlags<Sv39> = build_flags("__V");
        let ptr: NonNull<u8> = unsafe {
            KERNEL_SPACE
                .assume_init_ref()
                .translate(VAddr::new(vaddr), VALID)
                .unwrap()
        };
        ptr.as_ptr() as usize
    }
}

/// 帧缓冲物理地址（页对齐）。
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
    // fb 是 'static DMA 切片：经 KERNEL_SPACE 翻译得到其物理地址。
    let paddr = VirtioHal::virt_to_phys(fb.as_ptr() as usize);
    FB_PADDR.store(paddr, Ordering::SeqCst);
    FB_LEN.store(fb.len(), Ordering::SeqCst);
    FB_WIDTH.store(width, Ordering::SeqCst);
    FB_HEIGHT.store(height, Ordering::SeqCst);
    Mutex::new(Gpu(gpu))
});

/// 初始化 GPU 与键盘。
///
/// 必须在内核地址空间建立、`KERNEL_SPACE` 写入、MMIO 区域映射完成之后调用
/// （HAL 翻译与设备寄存器访问都依赖这些前置条件）。
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
