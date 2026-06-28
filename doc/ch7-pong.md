# ch7 · 用户态图形 Pong（管道 IPC 双人对战）实现笔记

在 ch7（Sv39 + 堆 + 块设备 + 管道/信号 IPC）上落地一个**用户态图形乒乓**，
真正用 ch7 的 **管道(pipe)** 在多个进程间传递对战状态。复用 ch4「黄金样板」
建立的 VirtIO-GPU / VirtIO-Input / 自定义 syscall 基建，仅改 MMIO 槽位与 HAL
地址翻译以适配 ch7。全部改动以 `feature = "game"` 门控，**不影响默认 `cargo run`
与判题器**（默认仍加载 initproc → user_shell）。

截帧产物：`scratchpad/ch7_pong.png` —— 上下墙 + 中线虚线 + 左(青)右(橙)两挡板
+ 白球 + 比分（`1 : 0`，证明球的物理/计分跑通、挡板位置确实经管道送达裁判）。

---

## 1. 进程结构与管道数据流（本实验的核心）

**1 裁判 + 2 玩家，4 条管道**（全部在 `fork` 之前由父进程创建，子进程经
`fd_table` 继承）：

```
                cmd_l (命令 1B)                      pos_l (挡板位置 2B)
 ┌─────────┐ ──────────────▶ ┌─────────┐ ◀────────────── ┌─────────┐
 │ 玩家 L   │                 │  裁判    │                 │ 玩家 L   │
 │ (左挡板) │ ◀────────────── │  / 渲染  │ ──────────────▶ │ (左挡板) │
 └─────────┘    cmd_l         │ (帧缓冲) │     pos_l        └─────────┘
 ┌─────────┐    cmd_r         │          │     pos_r        ┌─────────┐
 │ 玩家 R   │ ◀────────────── │          │ ◀────────────── │ 玩家 R   │
 └─────────┘ ──────────────▶ └─────────┘ ──────────────▶ └─────────┘
```

- **裁判进程**（父）：唯一持有帧缓冲（`framebuffer_info` 按需映射到
  `0x4000_0000`），唯一读键盘（`key_event`），跑球的物理/碰撞/计分/渲染/`gpu_flush`。
  它**不自己移动挡板**——挡板位置只能从管道得知。
- **玩家进程**（两个 `fork` 出的子进程）：各自在**独立地址空间**里维护自己挡板的
  Y 坐标，每帧从 `cmd_*` 收一条移动命令、更新挡板、再把**新挡板位置经 `pos_*`
  发回裁判**。

**每帧握手（lockstep，无竞争无死锁）：**

```
裁判:  poll 键盘 → 写 cmd_l, cmd_r ──唤醒两玩家──▶ 阻塞读 pos_l, pos_r
玩家:  阻塞读 cmd_x ──▶ 更新自己挡板 ──▶ 写 pos_x ──▶ 回到阻塞读
裁判:  两个 pos 都到齐 → 推进球+碰撞+计分 → 渲染 → flush → 节流 ~40fps
```

**管道在哪承载了对战数据：** 挡板位置（`pos_l`/`pos_r` 上的 `u16`）是球碰撞判定
的关键输入，而它**只**存在于玩家进程的地址空间——裁判要拿到它，唯一途径就是读这
两条管道。命令（`cmd_*`）反向流动，构成每帧一次的同步握手：管道既是**数据通道**
又是**帧同步原语**。单键盘是「破坏性弹出」的共享设备只能有一个读取者，故由裁判统
一读键盘、再经 `cmd_*` 把按键路由给对应玩家——按键也成了跨进程数据流。

操作：玩家 L = `I`(上)/`K`(下)，玩家 R = `↑`/`↓`，`Q` 退出（裁判退出→关闭命令
写端→玩家读到 EOF 一并退出，干净收摊）。

---

## 2. 改动文件一览

### ch7 内核 `tg-rcore-tutorial-ch7`
- `Cargo.toml`：加 `[features] game`（`virtio-drivers`/`spin` 本就是依赖）。
- `src/gpu.rs`（新）：移植自 ch4，两处适配——**MMIO 槽位** `0x1000_2000`（GPU=bus.1）；
  **HAL `virt_to_phys`** 改走 `KERNEL_SPACE.translate`（照 `virtio_block.rs`，非 ch4 恒等）。
  `fb_paddr` 也由 `virt_to_phys(fb.as_ptr())` 求得而非直接当恒等。
- `src/input.rs`（新）：键盘槽位 `0x1000_3000`（bus.2），与 GPU 共享同一 HAL。
- `src/main.rs`：`mod gpu/input`（feature 门控）；`kernel_space()` 增 GPU/键盘两页 MMIO 映射；
  syscall 初始化后 `gpu::init()` + `init_gpu`；**feature=game 时加载 `pong` 取代 `initproc`**；
  `impls` 加 `impl Gpu for SyscallContext`（`framebuffer_info` 按需映射帧缓冲）。
- `src/process.rs`：`Process::map_framebuffer(paddr,len)` —— `map_extern` 到 `0x4000_0000`，`U_WRV`。
- `build.rs`：`CARGO_FEATURE_GAME` 时改用 `cases.toml` 的 `[ch7_game]` 用例集。

### ch7 用户程序 `tg-rcore-tutorial-ch7/tg-rcore-tutorial-user`
- `Cargo.toml`：加 `[[bin]] pong`，并加 **`[patch.crates-io]` 把 syscall 指向本地路径**
  （否则用户从 crates.io 拉到的旧 syscall 没有 GPU/输入 wrapper；版本均 0.4.8，对齐）。
- `cases.toml`：新增 `[ch7_game] = ["pong"]`。
- `src/bin/pong.rs`（新）：无堆实现。建 4 管道 + 2 次 fork；玩家循环（`run_player`）与
  裁判循环（`run_referee`，含球物理/碰撞/计分 + 复用 ch4 3×5 数字字模渲染比分）。

### 共享 crate `tg-rcore-tutorial-syscall`
- **未改**：ch4 那个 agent 已扩展好 `FbInfo`/键码常量/`Gpu` trait/`init_gpu`/
  `framebuffer_info`/`gpu_flush`/`key_event`，ch7 直接复用。

---

## 3. ch7 与 ch4 的关键差异（移植要点）

1. **MMIO 槽位**：ch7 有块设备占 `bus.0`(0x1000_1000)，故 GPU=bus.1(0x1000_2000)、
   键盘=bus.2(0x1000_3000)。`kernel_space()` 的 MMIO 映射在 ch7 原本只覆盖块设备一页，
   feature=game 时补映射 GPU/键盘两页（satp 设置前）。
2. **HAL 地址翻译**：ch4 内核全恒等映射故 `virt_to_phys = vaddr`；ch7 有全局
   `KERNEL_SPACE`，GPU/Input HAL 复用块设备的 `KERNEL_SPACE.translate` 路径。
   （内核堆仍是恒等映射，所以翻译结果数值上等于虚址，但走页表更统一、更稳。）
3. **进程模型**：ch4 所有进程在启动时静态加载、逐个 `map_framebuffer`；ch7 只在启动加载一个
   init 进程，其余靠 `fork`/`exec` 动态创建。因此帧缓冲映射策略改为**按需**（见下）。
4. **初始进程替换**：ch7 默认从 fs.img 加载 `initproc`；feature=game 改加载 `pong`，
   由它 fork 出玩家。默认判题路径不变。

---

## 4. 按需映射帧缓冲（ch7 对 ch4 的优化）

ch7 的 `fork` 用 `AddressSpace::cloneself` **深拷贝**所有 `areas`：若像 ch4 那样在
`from_elf` 就把 ~300 页（640×480×4）的帧缓冲映射进进程，则每次 fork 会给子进程**复制
整屏像素**（≈1.2MB/子进程），既浪费又让玩家莫名其妙持有帧缓冲。

解法：把映射推迟到 `framebuffer_info` 系统调用首次被调用时——在内核里检查
`0x4000_0000` 是否已映射，未映射才 `Process::map_framebuffer`。由于裁判进程在
**fork 之后**才调用 `framebuffer_info`，玩家子进程根本没有这片映射，零浪费、语义也更干净
（只有真正渲染的进程持有帧缓冲）。

---

## 5. 构建与无头截帧

```bash
cd tg-rcore-tutorial-ch7
cargo build --features game            # build.rs 自动构建 [ch7_game] 的 pong 并打包 fs.img
K=target/riscv64gc-unknown-none-elf/debug/tg-rcore-tutorial-ch7
IMG=target/riscv64gc-unknown-none-elf/debug/fs.img

# 关键：保留 ch7 的块设备 bus.0，再加 GPU(bus.1)/键盘(bus.2)/monitor/serial
qemu-system-riscv64 -machine virt -bios none -kernel $K \
  -drive file=$IMG,if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-gpu-device,bus=virtio-mmio-bus.1,xres=640,yres=480 \
  -device virtio-keyboard-device,bus=virtio-mmio-bus.2 \
  -display none -monitor unix:mon.sock,server,nowait -serial file:ser.log &
sleep 5
for k in i i up up k down; do echo "sendkey $k" | socat - unix-connect:mon.sock; sleep 0.3; done
echo "screendump out.ppm" | socat - unix-connect:mon.sock
ffmpeg -y -i out.ppm out.png
```

> 注：默认 `cargo build`（无 game）会用标准 `[ch7]` 集重建 fs.img（含 initproc/user_shell），
> 覆盖游戏镜像。要再次截帧，重跑 `cargo build --features game` 即可。

---

## 6. 踩坑记录

1. **单键盘无法被两个玩家进程各自读**：VirtIO-Input 的 `pop_pending_event` 是破坏性弹出，
   两个读者会互相吃掉对方的按键。故由裁判统一读键盘，按键经 `cmd_*` 管道路由给玩家——
   这反而让设计更贴合「pipe 承载跨进程数据」的主题。
2. **阻塞 vs 非阻塞管道**：内核 `PipeReader::read` 在无数据时返回 `-2`，用户 `pipe_read`
   封装见 `-2` 即 `sched_yield` 重试（写端关闭返回 0=EOF）。正好支撑 lockstep：裁判先写两条
   命令再阻塞读两个位置，玩家先阻塞读命令再写位置，环形缓冲(32B)每帧仅过 1~2 字节，永不满，
   无死锁。
3. **fork 前建好全部管道**：4 条管道必须在两次 `fork` 之前创建，子进程才能继承到全部端；
   各进程再 `close` 掉自己用不到的端（保持 EOF 语义清晰）。
4. **fork 深拷贝帧缓冲**：见第 4 节，改用按需映射规避。
5. **用户 crate 默认拉 crates.io 的 syscall**：ch7 用户 `Cargo.toml` 原本无 `[patch.crates-io]`，
   需补上指向本地 syscall，pong 才能调用 `framebuffer_info/gpu_flush/key_event`。
6. **固定 640×480 几何**：渲染常量按 640×480 写死，QEMU 用 `xres=640,yres=480` 对齐；
   `put()` 仍做边界裁剪，分辨率不符也不会越界崩溃。

---

## 7. 回归

- `cargo build`（无 game）：ch7 默认链路 0 警告通过（`#![deny(warnings)]`），fs.img 重建为
  标准 `[ch7]` 集（含 initproc/user_shell/pipetest），判题路径不受影响。
- `cargo build --features game`：ch7 图形链路 0 警告通过，截帧见到完整 Pong 画面。
- 共享 syscall crate 未改动，ch3~ch8 既有 API 不受影响。
