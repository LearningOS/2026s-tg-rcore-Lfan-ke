# ch3 · 用户态图形贪吃蛇 实现笔记

在 ch3（**多道程序章：satp=Bare、无 Sv39 分页、无内核堆**）上落地一个用户态
图形贪吃蛇，复用 ch4 建好的 `tg_syscall` GPU 基建（`FbInfo` / `Gpu` trait /
`framebuffer_info`/`gpu_flush`/`key_event` / `KEY_*`），但驱动层因 Bare + 无堆
而与 ch4（Sv39 + 堆）完全不同。全部改动以 `feature = "game"` 门控，
**不影响默认判题路径**（已回归：`./test.sh base` ✓ 4/4、`./test.sh exercise` ✓ 7/7）。

截帧产物：`scratchpad/ch3_snake.png`（顶部分数 HUD + 网格 + 三节蛇身(亮头) + 食物）。

---

## 1. ch3 与 ch4 的三个本质差异（本章难点）

| 维度 | ch4（Sv39+堆） | ch3（Bare+无堆，本章） |
|------|----------------|------------------------|
| 帧缓冲给用户 | `map_framebuffer` 映射到用户 `0x4000_0000`（U_WRV） | **无需映射**：Bare 恒等，`fb_paddr` 即用户可直访的物理地址 |
| MMIO 访问 | 内核地址空间需 `map_extern` GPU/键盘寄存器 | **无需映射**：Bare 下 S 态直接访问 `0x1000_*` |
| DMA 分配 | HAL `dma_alloc = alloc_zeroed`（内核堆） | **静态 DMA 池 + bump 游标**（无堆） |
| 键盘 | VirtIO-Input（`bus.1`） | **UART 非阻塞读**（`virtio-drivers` 的 Input 要堆，本章无堆 → 走官方 ch3 思路） |

PMP（见 `tg-sbi/m_entry.asm`：`pmpcfg0=0x0f` TOR+RWX、L=0）对 S/U 都放行整块物理内存，
故 U 态贪吃蛇可直写帧缓冲、内核可直访 MMIO。

## 2. 无堆静态 DMA 池（核心）

`src/gpu.rs` 的 `VirtioHal`：
- DMA 不来自堆，而来自**固定物理地址保留区** `DMA_BASE = 0x8080_0000`、`DMA_SIZE = 6 MiB`。
- `dma_alloc(pages)`：`AtomicUsize` 游标 `fetch_add(pages<<12)`、清零、返回物理地址；
  越界 panic。`dma_dealloc` 空操作（设备生命周期内不回收）。
- `phys_to_virt`/`virt_to_phys` 恒等。

**为什么不用 `static mut DMA_POOL: [u8; N]`（.bss）**：本章用户程序被
`AppMeta::iter` 拷到固定物理地址 `0x8040_0000`（占 2 MiB）。内核 `.bss` 紧贴内核镜像
（`0x8020_0000` 之后），若把 1~4 MiB 的池塞进 `.bss`，会**越过 `0x8040_0000` 与用户程序区重叠**
被覆盖。改用 `0x8080_0000` 起的固定高地址区（在内核镜像 `__end≈0x8027_a000` 与用户区
`…0x8060_0000` 之上），与两者都不重叠。链接后核对：`__end=0x8027_a000`、
snake.bin≈45 KiB 装在 `0x8040_0000`、池 `0x8080_0000..0x80E0_0000`，三段不交叉。

设备默认上报 1280×800（帧缓冲≈3.9 MiB）；截帧时给 QEMU 传 `xres=640,yres=480`
缩小帧缓冲、加快每帧填充。snake **自适应分辨率**（以 `fb.width` 为行距、由宽高算网格），
两种分辨率都正确。

## 3. virtio-drivers 0.1.0 的链接期堆依赖

`virtio-drivers 0.1.0` 顶层 `extern crate alloc`，链接期**强制要求一个
`#[global_allocator]`**——即便本章只用 `VirtIOGpu`（其 DMA 全走上面的静态池、
`VirtIOInput` 才用 `Box`，本章根本不实例化它）。
解决：`src/game_alloc.rs` 提供一个 `feature=game` 门控的 **64 KiB bump 全局分配器**
（仅满足链接符号，GPU 路径运行时一字节都不会用到）。帧缓冲等大块内存仍来自静态 DMA 池，
“无（动态）堆”特性保持不变。base 路径不链 virtio、无此分配器，判题不受影响。

## 4. UART 键盘（`src/input.rs`）

`virtio-drivers` 的 `VirtIOInput` 用 `Box<[InputEvent;32]>` 需要真堆，与“无堆”冲突 →
按**官方 ch3-snake 思路**改读 16550 UART（`0x1000_0000`，与 `tg-sbi` 同址）：
- `read_byte()`：`LSR(+5).bit0` 数据就绪则读 `RBR(+0)`，非阻塞。
- 小状态机解析方向键转义序列 `ESC [ A/B/C/D`，并把 `WASD`/`IJKL`/`q` 映射为
  与 ch4 一致的 `KEY_*` evdev 码 → 用户游戏代码无差别。
- 内核 `Gpu::key_event` 转发到 `poll_key()`。
（已验证：通过 `-serial unix:` 注入 `sssdd`，内核读到 `rx=0x73/0x73/0x73/0x64/0x64`。）

## 5. 改动文件一览

内核 `tg-rcore-tutorial-ch3`：
- `Cargo.toml`：`[features] game = ["dep:spin","dep:virtio-drivers"]`（两者设为 optional，
  base 路径不编译）。
- `src/gpu.rs`（新）：静态 DMA 池 HAL + `VirtIOGpu`（`bus.0`=`0x1000_1000`）+ `fb_*`/`flush`。
- `src/input.rs`（新）：UART 非阻塞读 + 键码映射。
- `src/game_alloc.rs`（新）：仅 game 的 64 KiB bump 全局分配器（链接符号）。
- `src/main.rs`：`mod gpu/input/game_alloc`（cfg game）；`rust_main` 加 `gpu::init()` +
  `init_gpu`；`impls` 加 `impl Gpu`（Bare 下 `framebuffer_info` 直接把 `fb_paddr` 回填用户）。
- `build.rs`：`CARGO_FEATURE_GAME` → 选 `[ch3_game]` 用例集。

用户 `tg-rcore-tutorial-ch3/tg-rcore-tutorial-user`：
- `Cargo.toml`：加 `[[bin]] snake / fb_test` + `[patch.crates-io]` 指向本地 syscall（同 ch4，
  否则用户拿不到 GPU wrapper）。
- `cases.toml`：新增 `[ch3_game] base=0x8040_0000 step=0x20_0000 cases=["snake"]`。
- `src/bin/snake.rs`（新）：无堆定长实现（网格/蛇身坐标/食物/移动·吃·碰撞·计分、
  方向键/IJKL/WASD 转向、~6FPS by `get_time`、`sched_yield` 让出 CPU、3×5 数字字模画分数、
  自适应分辨率、撞墙/自咬即重开以长驻便于截帧）。游戏状态置于 `static`，避免占用
  内核给的 8 KiB 用户栈。
- `src/bin/fb_test.rs`（新）：阶段 2 帧缓冲通路自检（渐变+RGB 色块）。

跨 crate 未改 `tg-syscall`（ch4 已扩展好，ch3 直接复用）。

## 6. 构建与无头截帧

```bash
cd tg-rcore-tutorial-ch3
cargo build --features game
K=target/riscv64gc-unknown-none-elf/debug/tg-rcore-tutorial-ch3
qemu-system-riscv64 -machine virt -m 128M -bios none -kernel $K \
  -device virtio-gpu-device,bus=virtio-mmio-bus.0,xres=640,yres=480 \
  -display none -monitor unix:mon.sock,server,nowait -serial unix:uart.sock,server,nowait &
sleep 6
echo "screendump out.ppm" | socat - unix-connect:mon.sock
ffmpeg -y -i out.ppm ch3_snake.png
# 交互：printf 's' | socat - unix-connect:uart.sock   （wasd/ijkl/方向键/q）
```
阶段 2 可把 `[ch3_game]` 临时换成 `["fb_test"]` 验证 GPU+静态池通路。

## 7. 踩坑

1. **`.bss` 静态池会和用户程序加载地址 `0x8040_0000` 重叠** → 改用固定高地址保留区
   `0x8080_0000`（Bare 下当物理地址直接用）。
2. **virtio-drivers 0.1.0 链接期必须有 `#[global_allocator]`**（即便只用 GPU）→ 加一个
   game 门控的 bump 分配器顶替；别误以为“只用 GPU 就不需要堆”。
3. **设备默认 1280×800 而非 640×480**：snake 必须按 `fb.width` 取行距，否则错位花屏；
   截帧给 QEMU 传 `xres/yres` 缩小帧缓冲、提速。
4. **`VirtIOInput` 需堆** → 本章用 UART 输入绕开（堆只在第 2 条为链接符号而留）。
5. **同一 `target` 路径**：先后 `cargo build`(base)/`--features exercise`/`--features game`
   会互相覆盖同名内核 ELF；截帧前务必重新 `--features game` 构建，否则跑到的是上一次的内核。
6. **用户栈仅 8 KiB**（TCB 内 `[usize;1024]`）：蛇身坐标数组放 `static` 而非栈局部，避免溢出。
