//! 内核态七巧板（tangram）图形游戏（仅 `feature = "game"`）。
//!
//! # 为什么在内核态（S 态）直接渲染？
//!
//! 第一章是 rCore 教程里**最裸**的一章：S 态启动后只有 SBI 串口打印 + panic，
//! **没有 trap 入口、没有 U 态、没有进程、没有 syscall 框架**。其余四个图形游戏
//! （ch3/ch4/ch7/ch8）都把游戏放在 *用户态*，靠内核提供的 GPU/键盘 syscall 渲染；
//! 但在第一章若要走同样的路，得先从零补出最小 trap + U 态 + syscall 三件套，
//! 成本高且偏离「最小执行环境」这一教学主题。
//!
//! 因此本章选择 **(A) 内核态直接渲染**：第一章本就运行在 S 态裸机、复位即
//! `satp = Bare`（恒等映射），可直接 init VirtIO-GPU（[`crate::gpu`]，无堆静态 DMA 池）
//! → 在帧缓冲上用整数多边形填充画出七巧板 → flush。这最契合第一章「最小内核」的定位，
//! 也最快得到正确画面。「图形游戏放进 S 态内核」本身就是第一章独有的设计取舍与教学点。
//!
//! # 七巧板与渲染
//!
//! 经典七巧板 = 7 块：2 个大直角三角形 + 1 个中三角形 + 2 个小三角形 + 1 个正方形
//! + 1 个平行四边形，每块不同颜色，在此拼成一个**大正方形**（最经典的拼法）。
//! 切分坐标取自 4×4 单位正方形，每块均为凸多边形（三角形 / 正方形 / 平行四边形）。
//!
//! 无 FPU、无浮点：多边形填充用**整数叉积半平面判定**——遍历包围盒内每个像素，
//! 若它落在多边形所有有向边的同一侧（叉积同号，含边界）即填充。

use crate::gpu;

/// 颜色（`(R, G, B)`）。
type Rgb = (u8, u8, u8);

/// 16550 UART 基址（QEMU `virt` 平台 UART0；Bare 下 S 态可直接读）。
const UART_BASE: usize = 0x1000_0000;
/// 接收缓冲寄存器（RBR）偏移。
const UART_RBR: usize = 0;
/// 线路状态寄存器（LSR）偏移。
const UART_LSR: usize = 5;
/// LSR「数据就绪」（Data Ready）位。
const LSR_DATA_READY: u8 = 1;

/// QEMU `virt` 平台 `mtime` 频率（10 MHz），用于动画节拍。
const TIMEBASE_HZ: u64 = 10_000_000;

// ── 七巧板几何（4×4 单位正方形上的切分；坐标均为整数）──
//
//   A(0,0)──────────E? ───B(4,0)        记号：
//   │   L1      ╱ sB╲  │                 A,B,C,D 四角；O 中心(2,2)；
//   │       ╱ ───────╲ │ E(4,2)          E(4,2) 右边中点；F(2,4) 下边中点；
//   │    ╱   SQ   ╱ mC ╲│                 G(3,1)、Q(3,3)、H(1,3) 为对角线四等分点。
//   O ─────── Q ───────  C(4,4)
//   │ L2  ╲ sM ╱  ╲      │
//   │      ╲ ╱  PA  ╲    │
//   D(0,4)──H? ──F? ─────┘
//
// 7 块：L1/L2 大三角、mC 中三角、sB/sM 小三角、SQ 正方形、PA 平行四边形。
// （已离线验证：面积合计 16、满覆盖、无重叠。）

/// 切分用到的 10 个顶点（4×4 单位坐标）。
const PTS: [(i32, i32); 10] = [
    (0, 0), // 0 A
    (4, 0), // 1 B
    (4, 4), // 2 C
    (0, 4), // 3 D
    (2, 2), // 4 O 中心
    (4, 2), // 5 E 右边中点
    (2, 4), // 6 F 下边中点
    (3, 1), // 7 G
    (3, 3), // 8 Q
    (1, 3), // 9 H
];

/// 一块七巧板：顶点下标序列（凸多边形，逆/顺时针均可）。
struct Piece(&'static [usize]);

/// 7 块七巧板（拼成正方形），顺序与调色板槽位一一对应。
const PIECES: [Piece; 7] = [
    Piece(&[0, 1, 4]),    // 大三角 1（上）
    Piece(&[0, 4, 3]),    // 大三角 2（左）
    Piece(&[1, 5, 7]),    // 小三角（角 B）
    Piece(&[7, 5, 8, 4]), // 正方形
    Piece(&[5, 2, 6]),    // 中三角（角 C）
    Piece(&[4, 8, 9]),    // 小三角（中）
    Piece(&[3, 6, 8, 9]), // 平行四边形
];

/// 多套配色主题（每套 7 色，对应 7 块）。UART 按键切换主题。
const THEMES: [[Rgb; 7]; 3] = [
    // 主题 0：明快糖果色
    [
        (231, 76, 60),  // 红
        (243, 156, 18), // 橙
        (241, 196, 15), // 黄
        (46, 204, 113), // 绿
        (26, 188, 156), // 青
        (52, 152, 219), // 蓝
        (155, 89, 182), // 紫
    ],
    // 主题 1：冷色霓虹
    [
        (0, 200, 255),
        (0, 255, 200),
        (120, 255, 120),
        (90, 140, 255),
        (180, 100, 255),
        (255, 90, 200),
        (60, 220, 255),
    ],
    // 主题 2：暖色夕阳
    [
        (255, 94, 87),
        (255, 154, 60),
        (255, 206, 84),
        (255, 130, 120),
        (214, 93, 177),
        (155, 89, 182),
        (255, 175, 100),
    ],
];

/// 背景色。
const BG: Rgb = (18, 18, 28);

/// 帧缓冲画布：直写 [`gpu::fb_paddr`] 处像素（`BGRA8888`，内存序 `[B, G, R, A]`）。
struct Canvas {
    /// 帧缓冲首地址（Bare 下即物理地址）。
    base: *mut u8,
    /// 字节长度。
    len: usize,
    /// 宽（像素）。
    w: i32,
    /// 高（像素）。
    h: i32,
}

impl Canvas {
    /// 绑定到当前 GPU 帧缓冲。
    fn new() -> Self {
        Self {
            base: gpu::fb_paddr() as *mut u8,
            len: gpu::fb_len(),
            w: gpu::fb_width() as i32,
            h: gpu::fb_height() as i32,
        }
    }

    /// 写一个像素（越界 / 越长自动忽略）。
    #[inline]
    fn put(&self, x: i32, y: i32, c: Rgb) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let o = ((y * self.w + x) * 4) as usize;
        if o + 3 >= self.len {
            return;
        }
        // SAFETY: 偏移已做边界检查，落在帧缓冲内；Bare 下可直接写。
        unsafe {
            *self.base.add(o) = c.2; // B
            *self.base.add(o + 1) = c.1; // G
            *self.base.add(o + 2) = c.0; // R
            *self.base.add(o + 3) = 0xff; // A
        }
    }

    /// 用纯色填满整屏。
    fn clear(&self, c: Rgb) {
        for y in 0..self.h {
            for x in 0..self.w {
                self.put(x, y, c);
            }
        }
    }

    /// 填充一个**凸多边形**（顶点为已映射到像素的整数坐标）。
    ///
    /// 算法：遍历包围盒内每个像素中心，对多边形每条有向边求叉积
    /// `(b-a) × (p-a)`；若所有叉积同号（含 0，即落在边上）则该点在多边形内，填色。
    /// 凸多边形此判据成立，且边界取「含」避免相邻块之间出现 1px 缝隙。
    fn fill_convex(&self, poly: &[(i32, i32)], c: Rgb) {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, y) in poly {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        min_x = min_x.max(0);
        min_y = min_y.max(0);
        max_x = max_x.min(self.w - 1);
        max_y = max_y.min(self.h - 1);
        let n = poly.len();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let mut pos = false;
                let mut neg = false;
                for i in 0..n {
                    let (ax, ay) = poly[i];
                    let (bx, by) = poly[(i + 1) % n];
                    // 用 i64 防止叉积溢出（坐标可达千级）。
                    let cross = (bx - ax) as i64 * (y - ay) as i64
                        - (by - ay) as i64 * (x - ax) as i64;
                    if cross > 0 {
                        pos = true;
                    } else if cross < 0 {
                        neg = true;
                    }
                }
                if !(pos && neg) {
                    self.put(x, y, c);
                }
            }
        }
    }
}

/// 把一块七巧板的单位坐标映射到像素并填充。
///
/// `scale` 为单位长度对应的像素数，`(ox, oy)` 为 4×4 正方形左上角在屏上的像素原点。
fn draw_piece(cv: &Canvas, piece: &Piece, ox: i32, oy: i32, scale: i32, c: Rgb) {
    let mut poly = [(0i32, 0i32); 4];
    for (slot, &idx) in piece.0.iter().enumerate() {
        let (ux, uy) = PTS[idx];
        poly[slot] = (ox + ux * scale, oy + uy * scale);
    }
    cv.fill_convex(&poly[..piece.0.len()], c);
}

/// 渲染一帧七巧板（仅重绘 7 块；背景由调用方在循环外预先铺好）。
fn render(cv: &Canvas, ox: i32, oy: i32, scale: i32, theme: usize, offset: usize) {
    let palette = &THEMES[theme % THEMES.len()];
    for (i, piece) in PIECES.iter().enumerate() {
        // 颜色随 offset 缓慢轮转，产生「呼吸换色」的动画感。
        let c = palette[(i + offset) % 7];
        draw_piece(cv, piece, ox, oy, scale, c);
    }
}

/// 非阻塞读取一个 UART 字节；无数据返回 `None`。
fn uart_read() -> Option<u8> {
    // SAFETY: Bare 模式下 UART MMIO 物理地址可由 S 态直接易失读取。
    let lsr = unsafe { core::ptr::read_volatile((UART_BASE + UART_LSR) as *const u8) };
    if lsr & LSR_DATA_READY != 0 {
        Some(unsafe { core::ptr::read_volatile((UART_BASE + UART_RBR) as *const u8) })
    } else {
        None
    }
}

/// 忙等约 `ms` 毫秒（读 `time` CSR，无需中断 / 定时器配置）。
fn delay_ms(ms: u64) {
    let ticks = TIMEBASE_HZ / 1000 * ms;
    let start = riscv::register::time::read64();
    while riscv::register::time::read64().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// 经 SBI 打印一个字符串（第一章无 `println!`）。
fn print_str(s: &str) {
    for b in s.bytes() {
        tg_sbi::console_putchar(b);
    }
}

/// 经 SBI 打印一个十进制整数。
fn print_dec(mut n: u32) {
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    if n == 0 {
        tg_sbi::console_putchar(b'0');
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    for &b in &buf[i..] {
        tg_sbi::console_putchar(b);
    }
}

/// 运行七巧板游戏：初始化布局 → 铺背景 → 永久循环（换色动画 + UART 切主题）。
///
/// 本函数永不返回——第一章无任务调度，渲染完成后内核驻留在此循环，
/// 便于无头截帧反复观察画面。
pub fn run() -> ! {
    let cv = Canvas::new();
    print_str("[tangram] framebuffer ");
    print_dec(cv.w as u32);
    print_str("x");
    print_dec(cv.h as u32);
    print_str(", rendering classic tangram square (kernel-mode)\n");

    // 4×4 正方形居中、取屏幕短边的 4/5（对齐到 4 的倍数，使每单位为整数像素）。
    let side = (cv.w.min(cv.h) * 4 / 5) & !3;
    let scale = (side / 4).max(1);
    let side = scale * 4;
    let ox = (cv.w - side) / 2;
    let oy = (cv.h - side) / 2;

    // 背景只铺一次：7 块严丝合缝平铺整个正方形，后续每帧只重绘 7 块即可。
    cv.clear(BG);

    let mut theme = 0usize;
    let mut offset = 0usize;
    let mut frame: u64 = 0;
    loop {
        render(&cv, ox, oy, scale, theme, offset);
        gpu::flush();

        // 少量输入：任一按键切换下一套配色主题。
        if uart_read().is_some() {
            theme = (theme + 1) % THEMES.len();
        }

        delay_ms(40);
        frame += 1;
        // 每约 0.6 秒把配色向前轮转一格，形成缓慢的换色动画。
        if frame % 15 == 0 {
            offset = (offset + 1) % 7;
        }
    }
}
