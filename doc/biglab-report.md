# BigLab 任务三 总结报告：基于并扩展 ch1~ch8 内核的游戏应用

> 扩展实验实践：设计并实现 ch1~ch8 的游戏应用，基于并扩展对应内核以支持它们。
> 要求 ≥3 个（须含 ch1-tangram、ch8-doom，及与老师确认的自创游戏）——本次**做满 5 个**，覆盖 ch1/ch3/ch4/ch7/ch8。

## 一、成果总览

| 游戏 | 章节 | 体现的 OS 特性 | 截帧验证 |
| :-- | :-- | :-- | :-- |
| 七巧板 tangram | ch1 | 最小裸机内核（无 trap/U 态）内核态直接驱动 GPU | 7 色七巧板正方形 ✅ |
| 贪吃蛇 snake | ch3 | 多道程序 / satp=Bare / **无堆静态 DMA 池** | 网格+蛇+食物 ✅ |
| 俄罗斯方块 tetris | ch4 | Sv39 把 framebuffer 映射进用户地址空间（黄金样板） | 棋盘+方块+侧栏 ✅ |
| 乒乓 pong（自创） | ch7 | **管道 pipe IPC**：3 进程 + 4 管道双人对战 | 双挡板+球+比分 ✅ |
| DOOM 风 FPS doom | ch8 | **线程+Mutex** 分离逻辑/渲染 + easy-fs 读关卡 | 第一人称 3D 迷宫 ✅ |

全部经**无头截帧**（QEMU virtio-gpu → `screendump` PPM → PNG）实测渲染正确，且**默认判题路径全部不受影响**（游戏一律 gate 在 `feature = "game"` 后，base/exercise 测试照常通过）。**全程未 git commit**，改动留工作区。

## 二、统一基建设计（一次建好，ch3~ch8 复用）

把"用户态图形游戏"拆成三根支柱，集中到**共享 crate `tg-syscall`**：

1. **VirtIO-GPU framebuffer**：`virtio-drivers 0.1.0`（与各章已用的 `virtio_block` 同版，Hal 仅 4 方法）。`VirtIOGpu::new(MmioTransport@0x1000_x000)` → `setup_framebuffer()->&mut[u8]`（BGRA8888）→ 画完 `flush()`。
2. **键盘输入**：VirtIO-Input（`pop_pending_event`）或 UART 非阻塞读，映射成统一键码。
3. **自定义系统调用 2000/2001/2002 = `framebuffer_info` / `gpu_flush` / `key_event`**，写进 `tg-syscall` 的 trait + 用户 wrapper，ch3~ch8 全复用。

两套 HAL 按章节内存模型分流：

- **Bare 章（ch1/ch3）**：satp=Bare、**无堆**。DMA 不能用 `alloc`，自建**固定物理区 + AtomicUsize bump 游标**的静态 DMA 池（`DMA_BASE=0x8080_0000`, 6 MiB）；framebuffer 物理地址==虚拟地址，用户/内核直接访问。
- **Sv39 章（ch4/ch7/ch8）**：有堆。DMA 用 `alloc_zeroed`，`virt_to_phys` 走 `KERNEL_SPACE.translate`（照 `virtio_block.rs`）。framebuffer 物理页用 `map_extern(0x4000_0000, U_WRV)` **映射进用户 Sv39 空间**（U 位必须有）。

MMIO 槽位用 `bus=virtio-mmio-bus.N` 钉死：无块设备章（ch1/3/4）GPU=bus.0=`0x1000_1000`；有块设备章（ch7/8）GPU=bus.1=`0x1000_2000`、键盘 bus.2。

## 三、五个游戏的关键实现与踩坑（真实记录）

- **ch4-tetris（黄金样板）**：建立全部基建。坑：用户 crate 默认从 crates.io 取 `tg-syscall`，需 `[patch.crates-io]` 指向本地改过的 crate；path-patch 的 crate 不享受 lint 豁免，触发 `unsafe_op_in_unsafe_fn`；MMIO 必须在 `satp::set` 前映射否则驱动缺页。
- **ch7-pong（自创·管道 IPC）**：1 裁判 + 2 玩家（fork）+ 4 管道。玩家在**各自地址空间**持有挡板 Y，裁判**只能靠读管道**得到它做碰撞/计分——管道真正承载了跨进程对战数据；用阻塞 `pipe_read/write`（-2→yield 重试）做逐帧 lockstep 握手，协作调度下无死锁。坑：ch7 `fork` 深拷贝每个映射区，若像 ch4 那样在 `from_elf` 映射 1.2MB framebuffer 会被每个玩家进程拷一份 → 改成**仅裁判调 `framebuffer_info` 时按需映射**。
- **ch8-doom（射线投射 FPS）**：每屏幕列投一条射线 DDA 找墙距、画竖直墙条。坑：**内核没设 `sstatus.FS`，用户态用 f32 会触发非法指令** → 全程**整数定点** + 运行时建整数 sin/cos 查表（Bhaskara 有理式，π 自约去，无浮点）。关卡由 `build.rs` 写进 `fs.img`，游戏 `open("doom.map\0")`（**路径须 NUL 结尾**）读取。线程：`thread_create` 逻辑线程（输入+移动+碰撞）与主渲染线程共享玩家位姿，**阻塞 Mutex** 保护。
- **ch3-snake（无堆·最硬基建）**：固定区静态 DMA 池。坑：**virtio-drivers 0.1.0 即便只用 GPU 也强制 `#[global_allocator]`**（其 `VirtIOInput` 用 `Box`）→ 加一个 64KiB game-gated bump 分配器占位（GPU 路径实际不触发它）；设备**默认 1280×800 非 640×480** → 蛇渲染按 `fb.width` 自适应；输入改用 **UART**（VirtIO-Input 需堆）。保留了早先为判题加的 trace/SYSCALL_COUNT 逻辑不动。
- **ch1-tangram（最裸·内核态渲染）**：ch1 无 trap/U 态/进程，**选择 S 态内核直接渲染**（这本身是 ch1「最小执行环境」的教学点）。经典 7 块拼正方形，整数叉积半平面判定填充凸多边形（无 FPU），离线脚本先验证拼图面积和=16、无重叠。坑：`deny(warnings)` 把 `run()` 后的 `shutdown` 判为不可达 → cfg 互斥两个结尾；默认 4KiB 栈不够 GPU 初始化 → game 下提到 64KiB。

## 四、与 AI 合作的实现过程

### 4.1 交互方式：人定框架、AI 填实现、截图验收
本次是「**人类主控编排 + AI 子代理实现**」的分层协作：

1. **需求澄清**：先就模糊处用选项式提问对齐（DOOM 做到哪种程度→选「raycaster 而非移植 doomgeneric」；自创游戏选哪个→推荐 ch7-pong 管道对战）。
2. **设计先行（并行调研 workflow）**：派多个只读子代理**并行**精读 5 个内核的扩展点、virtio-drivers 的 GPU/Input API、无头截帧方案，综合成一份《biglab-design.md》蓝图。这一步并行安全（只读、不构建）。
3. **顺序实现（一次一个内核）**：按蓝图建议顺序 ch4（样板）→ch7→ch8→ch3→ch1，**每个游戏派一个子代理跑到底**（扩展内核+写游戏+构建+截帧自验+写 note+回报）。
4. **截图验收**：每个子代理把最终帧存成 PNG，主控**用读图能力亲眼看截帧**确认画面正确，再放行下一个。

### 4.2 定下的「铁律」（都是踩坑换来的）
- **一次只构建一个内核**：早期并行派 9 个重构建子代理 → 11Gi 内存被打满 → swap 抖动 → 构建慢 10 倍。改为严格顺序后内存稳定在 ~8.9Gi 空闲。
- **构建必须内联前台跑完**：有子代理把慢构建丢后台就结束回合 → 变「僵尸」反复唤醒却不收尾。铁律：构建用 `timeout` 包住、前台等完再返回。
- **绝不 commit / 游戏一律 gate 在 feature 后**：保证默认判题路径零回归。

### 4.3 遇到的问题/bug 与解决（精选）
| 问题 | 现象 | 解决 |
| :-- | :-- | :-- |
| 并行构建 OOM | 内存满、swap 抖动 | 顺序化，一次一构建 |
| 子代理后台僵尸 | 反复唤醒不收尾 | 内联构建铁律 |
| 用户态无 FPU | f32 触发非法指令(ch8) | 整数定点 + sin/cos 查表 |
| 无堆做 DMA | `alloc` 不可用(ch1/3) | 固定区静态 DMA 池 + bump 游标 |
| fork 深拷贝 fb | 每玩家拷 1.2MB(ch7) | 裁判按需映射 framebuffer |
| 缺 U 位 | 用户访问 fb 触发 PageFault(ch4+) | `map_extern` flags 用 `U_WRV` |
| virtio 0.1.0 强制全局分配器 | 只用 GPU 也链接报错 | 加 64KiB 占位 bump 分配器 |
| `open` 路径未 NUL 结尾 | 读错文件(ch8) | `"doom.map\0"` |
| 设备默认 1280×800 | 画面错位(ch3) | 按 `fb.width` 自适应 |
| 无头看不到画面 | 没有显示窗口 | QEMU `screendump`→PPM→PNG→读图 |

## 五、学习效果评估

### 5.1 知识与能力的提升
- **设备与驱动**：吃透 VirtIO（MMIO 发现、Hal/DMA、GPU framebuffer/scanout、Input 事件、用 `bus=` 钉死槽位地址），从「读块设备」推广到 GPU/键盘。
- **地址空间**：把"物理 framebuffer 映射进用户 Sv39 空间且带 U 位"打通——把 ch4 的页表/权限位从抽象变成「不映射就黑屏/缺页」的具体后果。
- **无堆系统编程**：在 Bare 内核里手写静态 DMA 池，理解"没有 malloc 时设备内存从哪来"。
- **无 FPU 图形数学**：整数定点 raycaster + 查表三角函数 + 叉积多边形填充——理解定点数与硬件浮点开关 `sstatus.FS`。
- **并发与 IPC**：用 ch7 pipe 做真·跨进程对战、用 ch8 线程+Mutex 分离逻辑/渲染——把"管道/线程"从测试用例变成游戏里看得见的协作。
- **工程方法**：无头环境的可视化验证闭环（截帧→读图→迭代）；大任务的"设计先行 + 顺序实现 + 资源纪律"编排。

### 5.2 与传统教学实验教程的对比
- **定性**：传统 rCore 实验产出是**串口文字 + 测试通过/失败**，内核工作"看不见"；本扩展让内核**渲染出会动的游戏**，学习动机与"我真的让它跑起来了"的正反馈显著更强；且每个游戏**精准绑定一个章节的招牌特性**（ch4 地址空间映射 / ch7 管道 / ch8 线程+fs），把抽象机制锚定到可见现象。
- **定量**：单章扩展约新增 2 个内核驱动文件（gpu/input）+ 1 个用户游戏（数百行）+ note；覆盖了传统教程**不涉及**的 GPU/framebuffer/无头图形验证/无堆 DMA/定点图形等主题；5 章一致复用同一套 syscall 基建，体现"组件化复用"。
- **代价/反思**：图形带来调试维度上升（看不见 → 必须搭截帧管线）；无 FPU/无堆等约束迫使回到更底层（定点、静态池），既是负担也是更深的理解。AI 协作把"读 5 个内核找扩展点"这种广度活并行化、把"一次一个内核构建"这种有资源约束的串行活纪律化，是这次能在合理时间做满 5 个的关键。

## 六、复现方式
每个游戏：在对应章节目录 `cargo build --features game` 生成内核（ch7/ch8 会顺带把游戏镜像/关卡打进 `fs.img`），再用带 `-device virtio-gpu-device`（+ 块设备章的 blk + 键盘）、`-display none -monitor unix:sock` 的 QEMU 启动，`screendump` 截帧。各章 `notes/chX-*.md` 有逐个的文件清单、踩坑与截帧命令。**重跑判题前用默认 `cargo build` / `./test.sh` 即可（game 在 feature 后，互不影响）。**
