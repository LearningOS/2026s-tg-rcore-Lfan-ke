//! 非阻塞键盘输入（第三章图形贪吃蛇基建）。
//!
//! 第四章用 VirtIO-Input，但 `virtio-drivers 0.1.0` 的 `VirtIOInput` 内部用
//! `Box<[InputEvent; 32]>`，需要全局堆分配器——这与第三章“无堆”相悖。
//! 因此本章按**官方 ch3-snake 思路**改从 16550 UART（QEMU `virt` 的 `0x1000_0000`）
//! 非阻塞读字符，把 `WASD` / `IJKL` / 方向键转义序列映射为 evdev 键码，
//! 复用与第四章一致的 `KEY_*` 常量，让用户态游戏代码无差别。
//!
//! Bare 模式下 UART MMIO 寄存器可被 S 态直接读写，无需建立映射。

use core::sync::atomic::{AtomicU8, Ordering};
use tg_syscall::{KEY_DOWN, KEY_LEFT, KEY_Q, KEY_RIGHT, KEY_UP};

/// 16550 UART 基址（QEMU `virt` 平台 UART0）。
const UART_BASE: usize = 0x1000_0000;
/// 接收缓冲寄存器（RBR）偏移。
const UART_RBR: usize = 0;
/// 线路状态寄存器（LSR）偏移。
const UART_LSR: usize = 5;
/// LSR “数据就绪”（Data Ready）位。
const LSR_DATA_READY: u8 = 1;

/// 方向键转义序列解析状态：0 = 普通，1 = 收到 ESC，2 = 收到 `ESC [`。
static ESC_STATE: AtomicU8 = AtomicU8::new(0);

/// 初始化键盘输入。
///
/// UART 由 SBI 固件预先配好，本章只读 RBR，无需额外初始化；此函数仅复位
/// 转义状态机，保持与 [`crate::gpu::init`] 一致的初始化入口形态。
pub fn init() {
    ESC_STATE.store(0, Ordering::SeqCst);
}

/// 非阻塞读取 UART 一个字节；无数据返回 `None`。
fn read_byte() -> Option<u8> {
    // SAFETY: Bare 模式下 UART MMIO 物理地址可由 S 态直接易失读取。
    let lsr = unsafe { core::ptr::read_volatile((UART_BASE + UART_LSR) as *const u8) };
    if lsr & LSR_DATA_READY != 0 {
        Some(unsafe { core::ptr::read_volatile((UART_BASE + UART_RBR) as *const u8) })
    } else {
        None
    }
}

/// 把一个普通字符映射为方向 / 退出键码（`WASD` / `IJKL` / `q`）。
fn map_letter(b: u8) -> Option<u16> {
    match b {
        b'w' | b'W' | b'i' | b'I' => Some(KEY_UP),
        b's' | b'S' | b'k' | b'K' => Some(KEY_DOWN),
        b'a' | b'A' | b'j' | b'J' => Some(KEY_LEFT),
        b'd' | b'D' | b'l' | b'L' => Some(KEY_RIGHT),
        b'q' | b'Q' => Some(KEY_Q),
        _ => None,
    }
}

/// 非阻塞读取一个“按下”按键，返回 evdev 键码；无可识别按键返回 `None`。
///
/// 排空 UART 接收缓冲并经一个小型状态机解析方向键转义序列
/// （`ESC [ A/B/C/D`），首个被识别的按键即返回。
pub fn poll_key() -> Option<u16> {
    while let Some(b) = read_byte() {
        match ESC_STATE.load(Ordering::SeqCst) {
            // 普通态：识别 ESC 起始或直接字母。
            0 => {
                if b == 0x1b {
                    ESC_STATE.store(1, Ordering::SeqCst);
                } else if let Some(code) = map_letter(b) {
                    return Some(code);
                }
            }
            // 已收到 ESC：期待 '['。
            1 => {
                ESC_STATE.store(if b == b'[' { 2 } else { 0 }, Ordering::SeqCst);
            }
            // 已收到 'ESC [':末字节决定方向。
            _ => {
                ESC_STATE.store(0, Ordering::SeqCst);
                let code = match b {
                    b'A' => KEY_UP,
                    b'B' => KEY_DOWN,
                    b'C' => KEY_RIGHT,
                    b'D' => KEY_LEFT,
                    _ => continue,
                };
                return Some(code);
            }
        }
    }
    None
}
