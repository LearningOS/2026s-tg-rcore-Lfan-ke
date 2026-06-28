# ch4 · 用户态图形 Tetris（黄金样板）实现笔记

在 ch4（Sv39 + ELF 进程 + 堆）上落地一个用户态图形俄罗斯方块，并建立
ch7/ch8 可直接复用的 **VirtIO-GPU / VirtIO-Input / 自定义 syscall** 基建。
全部改动以 `feature = "game"` 门控，**不影响默认 `cargo run` 与判题器**。

截帧验证产物：`scratchpad/ch4_tetris.png`（棋盘 + 多色落块栈 + 下落方块 +
侧栏 NEXT 预览 / 分数 / 消行数）。`scratchpad/ch4_fbtest.png` 为阶段 3 的
帧缓冲通路自检（渐变 + RGB 色块，验证 BGRA 通道序正确）。

---

## 1. 改动文件一览

### 共享 crate `tg-rcore-tutorial-syscall`（ch3~ch8 通用，**纯新增**）
- `src/syscall.h.in`：追加 `__NR_framebuffer_info 2000 / __NR_gpu_flush 2001 / __NR_key_event 2002`。
- `src/gpu.rs`（新）：跨态共享定义 —— `FbInfo{ptr,len,width,height}`、`EV_KEY`/`KEY_PRESS`、
  方向键 + IJKL + 空格/回车/Esc/Q 的 evdev 键码常量。不受 kernel/user feature 限制。
- `src/lib.rs`：`mod gpu; pub use gpu::*;`。
- `src/kernel/mod.rs`：新增 `trait Gpu`（`framebuffer_info`/`gpu_flush`/`key_event`，默认 `unimplemented!`）、
  `static GPU: Container<dyn Gpu>`、`pub fn init_gpu(..)`，并在 `handle()` 的 `match id` 末尾加 3 个转发分支。
- `src/user.rs`：用户态 wrapper `framebuffer_info()->FbInfo` / `gpu_flush()->isize` / `key_event()->isize`；
  并把 `native::syscall0..6` 里裸 `asm!` 包进 `unsafe{}`（见踩坑 ②）。

### ch4 内核 `tg-rcore-tutorial-ch4`
- `Cargo.toml`：加 `spin = "0.9"`、`virtio-drivers = "0.1.0"`、`[features] game`。
- `src/gpu.rs`（新）：`VirtioHal`（恒等）、`VirtIOGpu` 全局实例、`init()`、`flush()`、
  `fb_paddr()/fb_len()/fb_width()/fb_height()`。
- `src/input.rs`（新）：`VirtIOInput` 全局实例、`poll_key()->Option<u16>`（只回按下事件）。
- `src/main.rs`：`mod gpu/input`（feature 门控）；`kernel_space()` 增加 MMIO 映射；
  `rust_main` 在建立内核地址空间后、创建进程前 `gpu::init()`，并对每个进程 `map_framebuffer`；
  `schedule()` 加 `init_gpu`；`impls` 加 `impl Gpu for SyscallContext`。
- `src/process.rs`：`Process::map_framebuffer(paddr,len)` —— `map_extern` 到 `0x4000_0000`，`U_WRV`。
- `build.rs`：`CARGO_FEATURE_GAME` 时选用 `[ch4_game]` 用例集。

### ch4 用户程序 `tg-rcore-tutorial-ch4/tg-rcore-tutorial-user`
- `Cargo.toml`：加 `[[bin]] tetris / fb_test`，并加 **`[patch.crates-io]` 把 syscall 指向本地路径**（见踩坑 ①）。
- `cases.toml`：新增 `[ch4_game] = ["tetris"]`（自检阶段可临时换成 `["fb_test"]`）。
- `src/bin/tetris.rs`（新）：无堆实现（定长棋盘/方块表）。7 种方块 × 4 旋态、碰撞/锁定/消行/计分、
  IJKL+方向键输入、3×5 数字字模渲染分数、`get_time()` 控制重力(200ms)与帧率(33ms)。
- `src/bin/fb_test.rs`（新）：取 fb → 画渐变+色块 → flush → 死循环（阶段 3 通路自检）。

---

## 2. 关键数据流

```
用户 tetris ──framebuffer_info()──▶ 内核 impl Gpu::framebuffer_info
                                      translate 用户 FbInfo 指针(U_WV) → 写回 {0x4000_0000, len, 640,480}
用户直写 0x4000_0000 像素(BGRA8888) ── 该虚址映射到 GPU DMA 物理页(fb_paddr)
用户 gpu_flush() ─────────────────▶ 内核 VirtIOGpu::flush()(transfer_to_host_2d + resource_flush)
用户 key_event() ─────────────────▶ 内核 input::poll_key()(pop_pending_event, EV_KEY & value==1)
```

- fb DMA 由 `VirtioHal::dma_alloc = alloc_zeroed` 从内核堆分配；内核堆**恒等映射**，故
  `fb.as_ptr()`（内核虚址）即物理地址 `fb_paddr`，直接 `map_extern` 进用户 `0x4000_0000`。
- `0x4000_0000` 位于 ELF/堆（低地址）与用户栈（VPN `(1<<26)-2..1<<26`，即 VA ≈ `1<<38`）之间，互不重叠。

---

## 3. 跨 crate 可复用符号（供 ch7/ch8 直接复用）

`tg_syscall::` 下，无需再改共享 crate：
- 类型：`FbInfo`（`#[repr(C)] {ptr,len,width,height}`，`FbInfo::BPP=4`）。
- 常量：`EV_KEY=1`、`KEY_PRESS=1`、`KEY_UP/DOWN/LEFT/RIGHT`、`KEY_I/J/K/L`、`KEY_SPACE/ENTER/ESC/Q`。
- 内核：`trait Gpu` + `init_gpu(&'static dyn Gpu)` + 自动分发（syscall 2000/2001/2002）。
- 用户：`framebuffer_info()` / `gpu_flush()` / `key_event()`。

ch7/ch8 移植清单：复制 `gpu.rs`/`input.rs`，仅改 **MMIO 槽位常量**（ch7/ch8 有块设备 →
GPU=`0x1000_2000`、键盘=`0x1000_3000`），`map_framebuffer` 调用点照搬，`impl Gpu`+`init_gpu` 照搬；
各自用户 crate 同样需要 `[patch.crates-io]` 指向本地 syscall 才能调用 wrapper。

---

## 4. 踩坑记录

1. **用户 crate 默认从 crates.io 拉 syscall**，不是本地路径 → 新增的 GPU wrapper 对用户不可见。
   解决：在 ch4 用户 `Cargo.toml` 加 `[patch.crates-io] tg-rcore-tutorial-syscall = { path = "../../tg-rcore-tutorial-syscall" }`。
   （patch 版本号需与注册表版本一致：均 0.4.8。）
2. **路径 patch 不享受 cargo cap-lints**：注册表依赖会被 `--cap-lints allow` 压制警告，
   而 patch 成的本地路径 crate 不会。本地 `user.rs` 的 `native::syscall0..6` 在 unsafe fn 内裸用 `asm!`，
   edition 2024 + `#![deny(warnings)]` 下报 `unsafe_op_in_unsafe_fn`(E0133)。解决：把 `asm!` 包进 `unsafe{}`。
3. **ch4 无全局 `KERNEL_SPACE`**（ch8 有）→ HAL 的 `virt_to_phys` 不能照搬 `KERNEL_SPACE.translate`。
   改用恒等（`vaddr`）即可，因为 GPU/队列 DMA 全在恒等映射的内核堆里。
4. **ch4 内核地址空间默认不映射 MMIO**（0x1000_1000/0x1000_2000）→ 开 satp 后驱动访问设备寄存器缺页。
   解决：在 `kernel_space()` 内、`satp::set` 之前 `map_extern` 这两页（`_WRV`，内核专属）。
5. **fb 映射缺 U 位 → 用户 PageFault**：`map_framebuffer` 必须 `build_flags("U_WRV")`。
6. **初始化时序**：`gpu::init()` 须在 `kernel_space()`（MMIO 映射 + satp）之后、`Process::new` 之前
   （进程创建要拿 `fb_paddr`）。
7. **`VirtIOGpu<'a,H,T>` 带生命周期**，`setup_framebuffer(&mut self)->&mut[u8]` 借用 self →
   先取出 `as_ptr()`/`len()`（Copy 值）再把 gpu move 进全局 `Lazy<Mutex<..>>`。
8. **判题器不能被图形 app 卡死**：tetris 是死循环且默认运行器无 GPU 设备。
   故用 `feature=game` 门控全部图形代码，并把图形 app 放进**独立的 `[ch4_game]` 用例集**——
   默认 `cargo run`/判题不加载它们，互不影响。
9. **virtio-drivers 0.1.0 API 与蓝图一致**（以实际源码核对）：
   `VirtIOGpu::{new, resolution()->(u32,u32), setup_framebuffer()->&mut[u8](len=w*h*4,BGRA), flush()}`；
   `VirtIOInput::pop_pending_event()->Option<InputEvent{event_type:u16,code:u16,value:u32}>`，
   `EV_KEY=1`、`value==1` 为按下。`Hal` 仅 4 个非 unsafe 方法。

---

## 5. 构建与无头截帧

```bash
# 构建（含 game feature；build.rs 自动构建 [ch4_game] 里的 tetris）
cd tg-rcore-tutorial-ch4
cargo build --features game
K=target/riscv64gc-unknown-none-elf/debug/tg-rcore-tutorial-ch4

# 无头起 QEMU（GPU=bus.0, 键盘=bus.1）
qemu-system-riscv64 -machine virt -bios none -kernel $K \
  -device virtio-gpu-device,bus=virtio-mmio-bus.0,xres=640,yres=480 \
  -device virtio-keyboard-device,bus=virtio-mmio-bus.1 \
  -display none -monitor unix:mon.sock,server,nowait -serial file:ser.log &

sleep 3
# 注入按键玩几步（QEMU 键名：left/right/up/down/spc/i/j/k/l/q）
echo "sendkey left" | socat - unix-connect:mon.sock
echo "sendkey spc"  | socat - unix-connect:mon.sock
# 截帧 → PNG
echo "screendump out.ppm" | socat - unix-connect:mon.sock
ffmpeg -y -i out.ppm out.png
```

阶段验证：先把 `[ch4_game]` 改为 `["fb_test"]` 截帧确认渐变+RGB 色块（GPU+映射通路通），
再换回 `["tetris"]` 截帧确认棋盘。两张参考图见 `scratchpad/ch4_fbtest.png` 与 `scratchpad/ch4_tetris.png`。

## 6. 回归

- `cargo build`（无 game）：ch4 默认链路 0 警告通过，标准用例不受影响。
- ch3/ch5/ch6/ch7/ch8 内核（`TG_SKIP_USER_APPS=1`）均能编译 —— 共享 crate 改动纯新增，未破坏既有 API。
