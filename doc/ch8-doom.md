# ch8 · 用户态图形 DOOM 风射线投射 FPS（线程分离逻辑/渲染 + easy-fs 读关卡）

在 ch8（Sv39 + 堆 + 块设备 + **线程/Mutex/Semaphore/Condvar 并发**）上落地一个
Wolfenstein 式第一人称迷宫漫游（raycaster FPS），真正用 ch8 的**线程**把
「输入/逻辑」与「渲染」分离、用 ch8 的 **Mutex** 保护共享玩家状态、从 **easy-fs
读关卡迷宫**。复用 ch4/ch7「黄金样板」的 VirtIO-GPU / VirtIO-Input / 自定义
syscall 基建（ch8 的 MMIO 槽位、HAL 与 ch7 完全一致）。全部改动以
`feature = "game"` 门控，**不影响默认 `cargo run` 与判题器**（默认仍加载 initproc）。

截帧产物：
- `scratchpad/ch8_doom.png` —— 出生点（2.5, 8.5）朝东：纵深走廊、竖直墙条、左红
  右绿立柱、远处灰墙被距离雾化、上半天花板渐变 + 下半地板渐变、透视收敛到灭点。
- `scratchpad/ch8_doom_move.png` —— 注入按键（前进 ×3 + 左转 ×3）后的另一帧视角：
  视图明显改变（玩家在迷宫里前进并旋转），**证明输入→逻辑线程→Mutex 共享状态→
  渲染线程**整条并发链路打通。

---

## 1. 并发结构（本实验的 ch8 核心）

一个 DOOM 进程内**创建两个线程**，分工：

```
        key_event()                       framebuffer 直写 + gpu_flush()
  ┌───────────────┐   共享玩家状态 (px,py,ang)   ┌────────────────┐
  │  逻辑线程      │ ─── Mutex 保护的临界区 ───▶ │  渲染线程(主)   │
  │ 读键盘+移动转向│ ◀────────────────────────── │ 射线投射画 3D   │
  │ +墙体碰撞      │                              │ 帧 + 刷屏       │
  └───────────────┘                              └────────────────┘
```

- **逻辑线程**（`thread_create(logic_thread, 0)`）：独占读 VirtIO 键盘（`key_event`
  是破坏性弹出，只能一个读者），把按键解释为前进/后退/左右转，做墙体碰撞（贴墙
  滑动），更新共享玩家位姿 `(px, py, ang)`，约 60Hz。
- **渲染线程（主线程）**：持有帧缓冲（`framebuffer_info` 映射到 `0x4000_0000`），
  每帧在临界区内**拷一份**玩家位姿（临界区极小），再对屏幕每一列投射射线、定点
  步进找最近墙、画竖直墙条（高度 ∝ 1/距离），约 40fps `gpu_flush`。

**Mutex 在哪承载了共享数据**：玩家位姿 `(px, py, ang)` 是两个线程的唯一共享可变
状态——逻辑线程写、渲染线程读。两线程**同属一个进程、共享同一地址空间与帧缓冲
映射**，因此用 ch8 的 `mutex_create(true)` / `mutex_lock` / `mutex_unlock`（阻塞
互斥锁）把每次读/写都圈进临界区，无数据竞争。ch8 是协作式调度（线程在每次会
阻塞/挂起的系统调用边界切换），渲染线程的 `gpu_flush`/`sleep` 与逻辑线程的
`key_event`/`sleep` 都是系统调用，自然交替执行，Mutex 在争用时正确阻塞/唤醒。

操作：`↑/I` 前进、`↓/K` 后退、`←/J` 左转、`→/L` 右转、`Q/Esc` 退出（逻辑线程
置 `alive=false`，渲染线程下一帧读到后退出，主线程 `waittid` 回收逻辑线程）。

---

## 2. 关卡从 easy-fs 读（本实验的 ch8 文件系统体现）

- **打包**：ch8 内核 `build.rs` 在 `feature=game` 时，除了把 `doom` 应用打进
  `fs.img`，还**生成一张 16×16 迷宫**并作为普通文件 `doom.map` 写进同一镜像
  （`root_inode.create("doom.map") + write_at`）。
- **格式**（紧凑二进制）：`[0]=宽 W` `[1]=高 H` `[2..2+W*H]=行优先网格`，`0`=空地，
  非 0=墙（取值即“墙色 id”：1=外墙灰、2=立柱红、3=立柱绿）。
- **读取**：游戏启动 `open("doom.map\0", RDONLY)` + 循环 `read` 到 `Vec` 再解析。
  换关只需换文件，无需重编译游戏逻辑——这正是文件系统的价值。

---

## 3. 渲染：全整数定点 raycaster（关键踩坑）

**踩坑 ①（最大坑）：ch8 内核未给用户态开启 FPU。** 最初用 f32 写 raycaster，
一跑到第一条浮点指令就 `Exception(IllegalInstruction)` 崩溃（内核不设置
`sstatus.FS`、也不在上下文切换里保存 f 寄存器）。**解法：全程整数定点**，按
prompt 建议的“查表最稳”路线：

- 位置 `FP=16` 位小数（1 格 = `1<<16`）；角度 `ANG=1024` 等分一周（2 的幂，掩码取模）。
- **正余弦查找表运行时整数构建**：用 Bhaskara I 有理式 `sin(pπ)=16p(1−p)/(5−4p(1−p))`
  （π 全约掉，纯有理式，i64 即可），在 `main` 里（创建线程前）一次性建好 `SIN/COS`
  两张 `[i32; 1024]` 表（幅值 ±`1<<14`），之后只读、无锁。
- 每列：相机角偏移 `off = x·FOV/W − FOV/2`，射线角 `ang+off`；以 `1/16` 格为步长
  **定点步进**直到命中墙；`perp = dist·cos(off)` 消鱼眼；墙高 `H·ONE/perp`；墙色
  = 基色 × 朝向明暗（命中竖直/水平墙面）× 距离雾化；上半天花板、下半地板各按 y
  做整数渐变。无任何浮点。

---

## 4. 改动文件一览

### ch8 内核 `tg-rcore-tutorial-ch8`
- `Cargo.toml`：加 `[features] game`（`virtio-drivers`/`spin` 本就是依赖）。
- `src/gpu.rs`、`src/input.rs`（新）：从 ch7 原样移植——**MMIO 槽位** GPU=`0x1000_2000`
  (bus.1)、键盘=`0x1000_3000`(bus.2)；**HAL `virt_to_phys`** 走 `KERNEL_SPACE.translate`
  （照 `virtio_block.rs`）。ch8 与 ch7 这部分环境完全相同，零改动。
- `src/main.rs`：`mod gpu/input`（feature 门控）；`kernel_space()` 在 satp 前补映射
  GPU/键盘两页 MMIO；`init_sync_mutex` 之后 `gpu::init()` + `init_gpu`；**feature=game
  时加载 `doom` 取代 `initproc`**；impls 加 `impl Gpu for SyscallContext`（`framebuffer_info`
  经 `get_current_proc` 按需映射帧缓冲——帧缓冲属进程、为所有线程共享）。
- `src/process.rs`：`Process::map_framebuffer(paddr,len)` —— `map_extern` 到 `0x4000_0000`，`U_WRV`。
- `build.rs`：`CARGO_FEATURE_GAME` 时改用 `[ch8_game]` 用例集（含 doom），并把
  `doom_level_bytes()` 生成的 16×16 迷宫作为 `doom.map` 文件打进 `fs.img`。

### ch8 用户程序 `tg-rcore-tutorial-ch8/tg-rcore-tutorial-user`
- `Cargo.toml`：加 `[[bin]] doom`，并加 **`[patch.crates-io]` 把 syscall 指向本地路径**
  （否则用户从 crates.io 拉到的旧 syscall 没有 GPU/输入 wrapper；版本均 0.4.8，对齐）。
- `cases.toml`：新增 `[ch8_game] = ["doom"]`。
- `src/bin/doom.rs`（新）：全整数定点实现。整数 sin/cos 表（Bhaskara）、easy-fs 读关卡、
  逻辑线程（输入/移动/碰撞）+ 渲染线程（射线投射）、ch8 Mutex 保护共享玩家状态。

### 共享 crate `tg-rcore-tutorial-syscall`
- **未改**：ch4 那个 agent 已扩展好 `FbInfo`/键码常量/`Gpu` trait/`init_gpu`/
  `framebuffer_info`/`gpu_flush`/`key_event`，并已有 `thread_create`/`mutex_*` 等
  ch8 wrapper，doom 直接复用。

---

## 5. 与 ch7-pong 的关系（移植要点）

ch8 的 GPU/键盘/HAL/帧缓冲映射与 ch7 **完全一致**（同样 bus.0 块设备、bus.1 GPU、
bus.2 键盘、同样 `KERNEL_SPACE.translate`、同样 `0x4000_0000 U_WRV` 映射），故
`gpu.rs`/`input.rs` 原样照搬。三处差异：

1. **执行单元**：ch7 是“进程即线程”，用 **管道(pipe) 跨进程**传对战状态；ch8 拆出
   线程，用 **Mutex 跨线程**（同一地址空间）共享玩家状态——本游戏的并发主题正是
   ch8 的线程+同步原语，而非 ch7 的 IPC。
2. **帧缓冲映射归属**：ch8 帧缓冲属**进程**、为所有线程共享，`framebuffer_info` 经
   `get_current_proc` 按需映射一次，渲染线程写、逻辑线程不碰；ch7 是按需映射给
   “裁判”进程以规避 fork 深拷贝。本游戏 doom 不 fork，按需映射仍干净（只映射一次）。
3. **初始进程**：ch7 默认 initproc、game 换 pong；ch8 默认 initproc、game 换 doom。

---

## 6. 构建与无头截帧

```bash
cd tg-rcore-tutorial-ch8
cargo build --features game            # build.rs 自动构建 doom 并把 doom + doom.map 打进 fs.img
K=target/riscv64gc-unknown-none-elf/debug/tg-rcore-tutorial-ch8
IMG=target/riscv64gc-unknown-none-elf/debug/fs.img

# 关键：保留 ch8 块设备 bus.0 + fs.img，再加 GPU(bus.1)/键盘(bus.2)/monitor/serial
qemu-system-riscv64 -machine virt -bios none -kernel $K \
  -drive file=$IMG,if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-gpu-device,bus=virtio-mmio-bus.1,xres=640,yres=480 \
  -device virtio-keyboard-device,bus=virtio-mmio-bus.2 \
  -display none -monitor unix:mon.sock,server,nowait -serial file:ser.log &
sleep 4
# 玩几步（QEMU 键名：i/j/k/l 或 up/down/left/right）
for n in 1 2 3; do echo "sendkey i" | socat - unix-connect:mon.sock; sleep 0.12; done
echo "screendump out.ppm" | socat - unix-connect:mon.sock
ffmpeg -y -i out.ppm out.png
```

> 注：默认 `cargo build`（无 game）会用标准 `[ch8]` 集重建 fs.img（含 initproc/user_shell/
> 各并发测例），覆盖游戏镜像。要再次截帧，重跑 `cargo build --features game` 即可。

---

## 7. 踩坑记录

1. **用户态无 FPU → 浮点指令 IllegalInstruction**（见第 3 节）：内核不开 `sstatus.FS`、
   不保存 f 寄存器，用户态任何 f32/f64 指令直接非法指令崩溃。**全整数定点 + 运行时
   整数三角表**（Bhaskara 有理式，π 约掉无需浮点）解决。
2. **`open` 路径必须 NUL 结尾**：内核 `open` 逐字节读到 `\0` 为止、不靠长度。最初传
   `"doom.map"`（Rust 字面量不带 `\0`）→ 读越界拿到错误文件名 → `failed to load doom.map`。
   改成 `"doom.map\0"`（与 ch6/ch8 其它读文件 app 的 `"filea\0"` 一致）即可。
3. **帧缓冲属进程、线程共享**：`framebuffer_info` 用 `get_current_proc()`（而非
   `current()`/线程）取进程做按需映射；两个线程共享同一片 `0x4000_0000`，渲染线程
   写像素、逻辑线程从不触碰，天然无冲突。
4. **协作式调度下的线程交替**：ch8 在系统调用边界切换线程。渲染循环每帧有
   `gpu_flush`/`sleep`（系统调用），逻辑循环有 `key_event`/`sleep`，二者都频繁让出，
   Mutex 临界区只拷几个整数，永不长时间持锁，无饥饿、无死锁。
5. **`thread_create` 入口约定**：入口函数 `fn() -> isize`，参数经 a0 传入；线程**不重跑
   `_start`**（堆/控制台由主线程已初始化、地址空间共享），结尾必须 `exit()` 以便
   `waittid` 回收（照 ch8 `threads.rs`/`race_adder_mutex_blocking.rs` 惯例）。
6. **用户 crate 默认拉 crates.io 的 syscall**：ch8 用户 `Cargo.toml` 原本无
   `[patch.crates-io]`，需补上指向本地 syscall，doom 才能调用 framebuffer_info/gpu_flush/
   key_event/thread_create/mutex_*。
7. **固定 640×480 几何**：QEMU 用 `xres=640,yres=480` 对齐；`put()` 仍做边界裁剪，
   分辨率不符也不会越界崩溃。

---

## 8. 回归

- `cargo build`（无 game）：ch8 默认链路 0 警告通过（`#![deny(warnings)]`），fs.img 重建为
  标准 `[ch8]` 集（含 initproc/user_shell/threads/mutex/condvar/deadlock 等测例），
  判题路径不受影响。
- `cargo build --features game`：ch8 图形链路 0 警告通过，截帧见到 3D 第一人称迷宫。
- 共享 syscall crate 未改动，ch3~ch8 既有 API 不受影响。
</content>
</invoke>
