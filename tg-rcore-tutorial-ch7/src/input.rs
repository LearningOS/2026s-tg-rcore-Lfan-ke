//! VirtIO-Input 键盘驱动（第七章图形游戏基建）。
//!
//! 与 GPU 共用 [`crate::gpu::VirtioHal`]。提供非阻塞轮询：每次返回一个
//! “按下”（evdev `EV_KEY`，value==1）事件的键码，供 `key_event` 系统调用使用。
//!
//! 第七章块设备占 `virtio-mmio-bus.0`、GPU 占 `bus.1`，故键盘落在 `bus.2`
//! （`0x1000_3000`）。

use crate::gpu::VirtioHal;
use core::ptr::NonNull;
use spin::{Lazy, Mutex};
use tg_syscall::{EV_KEY, KEY_PRESS};
use virtio_drivers::{MmioTransport, VirtIOHeader, VirtIOInput};

/// 键盘设备 MMIO 槽位（第七章 → `virtio-mmio-bus.2`）。
const VIRTIO_INPUT: usize = 0x1000_3000;

/// VirtIO-Input 设备封装。
struct Input(VirtIOInput<VirtioHal, MmioTransport>);

// SAFETY: 全局实例由 Mutex 串行化访问。
unsafe impl Send for Input {}

/// 全局键盘实例（延迟初始化）。
static INPUT: Lazy<Mutex<Input>> = Lazy::new(|| {
    let transport = unsafe {
        MmioTransport::new(NonNull::new(VIRTIO_INPUT as *mut VirtIOHeader).unwrap())
            .expect("virtio-input: invalid MMIO transport")
    };
    Mutex::new(Input(
        VirtIOInput::new(transport).expect("virtio-input: init failed"),
    ))
});

/// 初始化键盘设备。
pub fn init() {
    Lazy::force(&INPUT);
}

/// 非阻塞读取一个“按下”按键，返回 evdev 键码；无事件返回 `None`。
///
/// 丢弃松开/重复等非按下事件，只把按下事件透传给游戏。
pub fn poll_key() -> Option<u16> {
    let mut input = INPUT.lock();
    while let Some(event) = input.0.pop_pending_event() {
        if event.event_type == EV_KEY && event.value == KEY_PRESS {
            return Some(event.code);
        }
    }
    None
}
