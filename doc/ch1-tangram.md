# ch1 · 内核态七巧板（tangram）实现笔记

在 ch1（**最小裸机内核**：S 态启动后只有 SBI 串口打印 + panic，**无 trap / U 态 /
进程 / syscall 框架**）上落地一个图形七巧板游戏。这是五个游戏里最「从零」的一个：
ch3/4/7/8 都把游戏放在用户态、靠内核 syscall 渲染，而 ch1 连 trap 都没有。
全部改动以 `feature = "game"` 门控，**不影响默认判题路径**
（已回归：`./test.sh` 仍打印 `Hello, world!` ✓）。

截帧产物：`scratchpad/ch1_tangram.png`——一个由 **7 块不同色多边形**严丝合缝拼成的
经典七巧板**正方形**（红/橙 两大三角、黄/蓝 两小三角、绿 正方形、青 中三角、紫 平行四边形）。

---

## 1. 关键抉择：选 (A) 内核态（S 态）直接渲染

ch1 极裸，要让七巧板「跑起来」有两条路：

- **(A) 内核态直接渲染（采用）**：ch1 本就在 S 态裸机、复位即 `satp = Bare`（恒等映射），
  可直接 init VirtIO-GPU（照 ch3 的无堆静态 DMA 池 HAL）→ 在帧缓冲上用整数多边形填充
  画七巧板 → flush。**不需要建 trap / U 态 / syscall**。
- (B) 从零补最小 trap + U 态 + syscall 再跑用户态 tangram：成本高、易损上下文，偏离
  「最小执行环境」这一教学主题。**未采用**。

「图形游戏放进 S 态内核」本身就是 ch1 独有的设计取舍与教学点：第一章的定位是
*最小执行环境*，渲染逻辑放在内核里反而最贴切，也最快得到正确画面。

## 2. 无堆静态 DMA 池（核心，照搬 ch3）

`virtio-drivers 0.1.0` 要 DMA 内存，但 ch1 没有内核堆。`src/gpu.rs` 的 `VirtioHal`：

- DMA 不来自堆，而来自**固定物理地址保留区** `DMA_BASE = 0x8080_0000`、`DMA_SIZE = 6 MiB`。
- `dma_alloc(pages)`：`AtomicUsize` 游标 `fetch_add(pages<<12)`、清零、返回物理地址；越界 panic。
- `phys_to_virt` / `virt_to_phys` 恒等（Bare）。
- 比 ch3 更省心：ch1 **没有任何用户程序**（无 `AppMeta` 加载），内核镜像 `~0x8020_0000`
  之上整片 RAM 都空着，DMA 池放 `0x8080_0000` 与内核镜像绝不重叠。

GPU 占第一个 VirtIO-MMIO 槽位 `virtio-mmio-bus.0`（`0x1000_1000`，ch1 无块设备）。
帧缓冲 `BGRA8888`（内存序 `[B,G,R,A]`），分辨率由设备上报（截帧给 QEMU 传 `xres=640,yres=480`）。

### virtio-drivers 0.1.0 的链接期堆依赖
该 crate 顶层 `extern crate alloc`，链接期**强制要求一个 `#[global_allocator]`**，
即便只用 `VirtIOGpu`（其 DMA 全走上面的静态池）。`src/game_alloc.rs` 提供一个
`feature=game` 门控的 64 KiB bump 全局分配器，仅满足链接符号、运行时一字节不用。
默认判题路径不链 virtio、无此分配器，判题不受影响。

## 3. 七巧板几何与整数多边形填充

`src/tangram.rs`。七巧板切分坐标取自 **4×4 单位正方形**（全整数顶点）：

```
A(0,0) B(4,0) C(4,4) D(0,4)  四角；O(2,2) 中心；
E(4,2) 右中点；F(2,4) 下中点；G(3,1) Q(3,3) H(1,3) 对角线四等分点。
7 块：L1[A,B,O] L2[A,O,D] 两大三角、mC[E,C,F] 中三角、
      sB[B,E,G] sM[O,Q,H] 两小三角、SQ[G,E,Q,O] 正方形、PA[D,F,Q,H] 平行四边形。
```

这套切分**离线验证过**（`scratchpad/verify.py`）：7 块面积合计 16（=4+4+2+1+2+1+2）、
满覆盖、无重叠——是货真价实的经典七巧板正方形拼法。

**填充原语**：无 FPU、无浮点。`fill_convex` 对任意凸多边形（三角形 / 正方形 / 平行四边形
统一处理）用**整数叉积半平面判定**——遍历包围盒内每像素，若它落在多边形所有有向边
的同一侧（`(b-a)×(p-a)` 同号，用 i64 防溢出）即填充。边界取「含」（同号含 0），
让相邻块共享边都被覆盖，**避免 1px 缝隙**。布局自适应分辨率：取屏幕短边 4/5、
对齐到 4 的倍数令每单位为整数像素、居中。

## 4. 少量输入 + 动画

- **换色动画**：背景只铺一次（7 块平铺整个正方形，后续每帧只重绘 7 块即可），
  每帧把配色向前轮转一格（约 0.6 s 一步），呈「呼吸换色」的生动感。节拍读 `time` CSR
  忙等，无需配置时钟中断。
- **UART 输入（可选）**：非阻塞读 16550 UART（`0x1000_0000`，Bare 下 S 态直读），
  任一按键切换下一套配色主题（共 3 套）。体现 ch1 也能做「少量输入」。

## 5. 改动文件一览

- `Cargo.toml`：`[features] game = ["dep:spin","dep:virtio-drivers","dep:riscv"]`（三者 optional，
  默认判题路径不编译）。
- `src/gpu.rs`（新）：无堆静态 DMA 池 HAL + `VirtIOGpu`（`bus.0`=`0x1000_1000`）+ `fb_*`/`flush`。
- `src/game_alloc.rs`（新）：仅 game 的 64 KiB bump 全局分配器（链接符号）。
- `src/tangram.rs`（新）：七巧板几何 + 整数叉积凸多边形填充 + 换色动画 + UART 切主题 + `run()` 主循环。
- `src/main.rs`：`mod gpu/game_alloc/tangram`（cfg game）；game 下 `_start` 栈 4 KiB→64 KiB；
  `rust_main` 打印 `Hello, world!` 后 `gpu::init()` + `tangram::run()`（永不返回）。
  两条收尾路径用 cfg 互斥，避免 game 下 `shutdown` 成「不可达代码」触发 `deny(warnings)`。

未碰 `tg-syscall`（(A) 方案内核态渲染不需要 syscall）。

## 6. 构建与无头截帧（ch1 无块设备）

```bash
cd tg-rcore-tutorial-ch1
cargo build --features game        # 默认路径用 `cargo build`（不带 feature）
K=target/riscv64gc-unknown-none-elf/debug/tg-rcore-tutorial-ch1
qemu-system-riscv64 -machine virt -m 128M -bios none -kernel $K \
  -device virtio-gpu-device,bus=virtio-mmio-bus.0,xres=640,yres=480 \
  -display none -monitor unix:mon.sock,server,nowait -serial file:ser.log &
sleep 4
echo "screendump out.ppm" | socat - unix-connect:mon.sock
sleep 1; ffmpeg -y -i out.ppm ch1_tangram.png
# 交互：printf ' ' | socat - unix-connect:...（需把 -serial 换成 unix: 才能注入）
```

## 7. 踩坑

1. **`deny(warnings, missing_docs)`（仅 riscv64）**：所有新 pub 项都要写 `///` 文档；
   game 下 `tangram::run()` 返回 `!`，其后的 `shutdown` 会变「不可达代码」告警 →
   把 game / 非 game 两条收尾用 `#[cfg]` 互斥，而非顺序排列。
2. **ch1 默认栈仅 4 KiB**：VirtIO-GPU 初始化在 S 态内核栈上跑，4 KiB 偏紧 →
   game 下把 `_start` 的 `STACK_SIZE` 提到 64 KiB（cfg 门控，不动默认路径）。
3. **virtio-drivers 0.1.0 链接期必须有 `#[global_allocator]`**（即便只用 GPU）→
   加一个 game 门控的 bump 分配器顶替；别误以为「只用 GPU 就不需要堆」。
4. **同一 `target` 路径**：`cargo build`(默认) 与 `--features game` 会互相覆盖同名内核 ELF；
   截帧前务必重新 `--features game` 构建。
5. **七巧板切分别凭印象拍脑袋**：平行四边形那一块极易写成「第二个正方形」或退化三角形 →
   先用脚本验证面积/覆盖/无重叠（`scratchpad/verify.py`），确认是真·七巧板再落代码。
