//! 图形 / 输入子系统的**跨态共享定义**。
//!
//! 这里只放在内核态与用户态之间需要保持二进制一致的类型和常量
//! （帧缓冲信息结构、evdev 键码），因此不受 `kernel` / `user`
//! feature 限制，ch4~ch8 均可直接复用。
//!
//! 配套的三个自定义系统调用号见 `syscall.h.in`：
//!
//! | 号   | 名称              | 语义                              |
//! |------|-------------------|-----------------------------------|
//! | 2000 | framebuffer_info  | 查询帧缓冲虚址 / 长度 / 分辨率    |
//! | 2001 | gpu_flush         | 把帧缓冲内容推送到屏幕            |
//! | 2002 | key_event         | 非阻塞读取一个按键 evdev 键码     |

/// 帧缓冲信息。
///
/// `framebuffer_info` 系统调用经用户传入的指针填回本结构：用户据此拿到
/// 帧缓冲在自身地址空间的虚址、字节长度与分辨率，然后直接按
/// `BGRA8888`（内存序 `[B, G, R, A]`）写像素。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FbInfo {
    /// 帧缓冲在用户地址空间的虚拟地址（Sv39 章节固定 `0x4000_0000`）。
    pub ptr: usize,
    /// 帧缓冲字节长度（`width * height * 4`）。
    pub len: usize,
    /// 屏幕宽度（像素）。
    pub width: u32,
    /// 屏幕高度（像素）。
    pub height: u32,
}

impl FbInfo {
    /// 每像素字节数（固定 `BGRA8888`）。
    pub const BPP: usize = 4;
}

/// evdev 事件类型：按键（`EV_KEY`）。
pub const EV_KEY: u16 = 1;
/// evdev 按键值：按下。
pub const KEY_PRESS: u32 = 1;

/// 方向键 ↑。
pub const KEY_UP: u16 = 103;
/// 方向键 ↓。
pub const KEY_DOWN: u16 = 108;
/// 方向键 ←。
pub const KEY_LEFT: u16 = 105;
/// 方向键 →。
pub const KEY_RIGHT: u16 = 106;

/// 字母键 I（上 / 旋转备用）。
pub const KEY_I: u16 = 23;
/// 字母键 J（左）。
pub const KEY_J: u16 = 36;
/// 字母键 K（下）。
pub const KEY_K: u16 = 37;
/// 字母键 L（右）。
pub const KEY_L: u16 = 38;

/// 空格键。
pub const KEY_SPACE: u16 = 57;
/// 回车键。
pub const KEY_ENTER: u16 = 28;
/// Esc 键。
pub const KEY_ESC: u16 = 1;
/// 字母键 Q（退出）。
pub const KEY_Q: u16 = 16;
