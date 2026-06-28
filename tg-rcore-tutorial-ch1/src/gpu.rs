//! VirtIO-GPU 帧缓冲驱动（第一章七巧板游戏基建）。
//!
//! 第一章是 **最小裸机内核**：S 态启动后只有 SBI 串口打印 + panic，
//! **没有 trap / U 态 / 进程 / syscall 框架**。因此本游戏选择在 **S 态内核里直接渲染**
//! （详见 [`crate::tangram`] 顶部的设计取舍），GPU 基建照搬第三章「无堆静态 DMA 池」做法：
//!
//! 1. **无块设备** → GPU 占用第一个 VirtIO-MMIO 槽位 `virtio-mmio-bus.0`
//!    （`0x1000_1000`）。本章 `satp` 复位即 Bare（恒等映射），MMIO 寄存器可被 S 态直接访问。
//! 2. **无堆分配器** → 不能用 `alloc_zeroed` 给 DMA 分配内存，改用一段
//!    **固定物理地址的静态 DMA 池** + bump 游标分配（见 [`VirtioHal`]）。
//!
//! 帧缓冲像素格式 `BGRA8888`（内存序 `[B, G, R, A]`），分辨率由设备上报。
//! Bare 模式下帧缓冲物理地址 == 内核虚址，故内核可直接写像素。

use core::{
    ptr::NonNull,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};
use spin::{Lazy, Mutex};
use virtio_drivers::{Hal, MmioTransport, VirtIOGpu, VirtIOHeader};

/// GPU 设备 MMIO 槽位（第一章无块设备 → `virtio-mmio-bus.0`）。
const VIRTIO_GPU: usize = 0x1000_1000;

/// 物理页位宽（4 KiB 页）。
const PAGE_BITS: usize = 12;

/// 静态 DMA 池基地址（物理地址）。
///
/// 选在 RAM 偏移 8 MiB 处：第一章内核镜像位于 `0x8020_0000` 附近且**无任何用户程序**，
/// 故本池远在内核镜像之上、与之互不重叠。因复位即 Bare，本物理地址同时就是
/// 内核可直接访问的虚址。
const DMA_BASE: usize = 0x8080_0000;
/// 静态 DMA 池容量（6 MiB）。
///
/// 需容纳帧缓冲与若干 VirtIO 队列页：640×480×4 ≈ 1.18 MiB；即便设备默认
/// 上报 1280×800（≈ 3.9 MiB）也放得下，留足余量。
const DMA_SIZE: usize = 6 << 20;

/// DMA 池 bump 游标（下一个可分配物理地址）。
static DMA_CURSOR: AtomicUsize = AtomicUsize::new(DMA_BASE);

/// VirtIO HAL：基于固定物理地址静态池的 bump 分配器。
///
/// 第一章无内核堆，故 DMA 内存来自 [`DMA_BASE`] 起的保留物理区间。
/// 因 Bare 模式恒等映射，`phys_to_virt` / `virt_to_phys` 均为恒等变换。
pub struct VirtioHal;

impl Hal for VirtioHal {
    /// 从静态 DMA 池 bump 出 `pages` 个连续物理页（已清零）。
    ///
    /// 池耗尽时 panic —— 表示 [`DMA_SIZE`] 估算不足，应调大。
    fn dma_alloc(pages: usize) -> usize {
        let bytes = pages << PAGE_BITS;
        // 游标按页对齐推进；DMA_BASE 已对齐，bytes 为页整数倍，故结果天然对齐。
        let paddr = DMA_CURSOR.fetch_add(bytes, Ordering::SeqCst);
        assert!(
            paddr + bytes <= DMA_BASE + DMA_SIZE,
            "virtio dma pool exhausted: need {bytes} bytes at {paddr:#x}"
        );
        // 清零（设备约定 DMA 区初始为 0；HAL 不保证底层 RAM 已清）。
        // SAFETY: [paddr, paddr+bytes) 在保留 DMA 池内，Bare 模式下可直接写。
        unsafe { core::ptr::write_bytes(paddr as *mut u8, 0, bytes) };
        paddr
    }
    /// 释放物理页：bump 分配器不回收（设备生命周期内不释放），空操作。
    fn dma_dealloc(_paddr: usize, _pages: usize) -> i32 {
        0
    }
    /// 物理地址 → 虚拟地址（Bare 恒等）。
    fn phys_to_virt(paddr: usize) -> usize {
        paddr
    }
    /// 虚拟地址 → 物理地址（Bare 恒等）。
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

/// 初始化 GPU。
///
/// Bare 模式无需提前映射 MMIO，直接探测设备即可。初始化后帧缓冲信息可经
/// [`fb_paddr`] / [`fb_len`] / [`fb_width`] / [`fb_height`] 查询。
pub fn init() {
    Lazy::force(&GPU);
}

/// 帧缓冲物理地址（页对齐，Bare 下即内核可直接访问的虚址）。
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
