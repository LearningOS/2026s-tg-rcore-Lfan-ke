# 大实验设计 · 共享图形游戏基建蓝图 + 每游戏实现清单

> 目标：在 tg-rcore 教学内核 ch1~ch8 上分层引入「用户态图形游戏」。
> 一套基建（VirtIO-GPU 取帧缓冲、VirtIO-Input/UART 取键盘、自定义 syscall 2000/2001/2002），
> 每章落一个能跑、能截帧验证的游戏。
> 本文为**只读调研综合**，供后续实现参考；不含任何已构建/已改动代码。

## 0. 章节 → 游戏 → 内核能力 映射

| 章 | 游戏 | satp | 堆 | 输入建议 | 帧缓冲访问模型 |
|----|------|------|----|---------|----------------|
| ch1 | tangram（七巧板） | Bare（裸物理） | 无 | UART getchar | 物理直访（VA=PA），固定 0x8040_0000 或 GPU DMA 区 |
| ch3 | snake（贪吃蛇） | Bare（恒等） | 无 | UART getchar | 内核读写 GPU；用户经 syscall 拿物理 fb |
| ch4 | tetris（俄罗斯方块） | Sv39 | 有 | UART 或 VirtIO-Input | fb 映射到用户固定虚址 0x4000_0000（带 U 位） |
| ch7 | pong（弹球对战） | Sv39 | 有 | VirtIO-Input | 同 ch4；已有 IPC/signal，可双玩家进程 |
| ch8 | doom（光线投射 demo） | Sv39 | 有 | VirtIO-Input | 同 ch4；多线程：逻辑线程 + 渲染线程共享 fb（Mutex） |

ch2/ch5/ch6 不强制配游戏，但基建（syscall crate、HAL）对它们透明可用。

---

## A. 统一基建

### A.1 virtio-drivers 版本选定：**0.1.0**

理由（与 ch8 现有 `virtio_block.rs` 完全一致，迁移成本最低）：
- `Hal` trait 是**非 unsafe、仅 4 个方法**（`dma_alloc/dma_dealloc/phys_to_virt/virt_to_phys`），
  与现仓库块设备 HAL 同款；0.7.5/0.13.0 改成 unsafe + 5 方法（`share/unshare/mmio_phys_to_virt`、
  `dma_alloc` 返回 `(PhysAddr, NonNull<u8>)`），需要重写。
- 全仓 ch3~ch8 的 `Cargo.toml` 已统一 `virtio-drivers = "0.1.0"`，**不要混版本**。
- 本机已缓存 0.1.0 / 0.7.5 / 0.13.0；0.1.0 edition 2018，`MmioTransport::new(NonNull<VirtIOHeader>)` 接口三版相同。

```toml
# 每章 Cargo.toml 已有 virtio-drivers = "0.1.0"；新增：
# （bitflags/log 已被 virtio-drivers 传递依赖，spin 各章已有）
```

GPU/Input 类型来自同一 crate：`virtio_drivers::{VirtIOGpu, VirtIOInput, VirtIOHeader, MmioTransport, Hal}`。

### A.2 MMIO 设备布局与 QEMU runner（关键：槽位决定地址）

QEMU `virt` 平台 virtio-mmio 槽位地址 = `0x1000_1000 + N*0x1000`，N=0..7。
**地址不是固定的，取决于 `bus=virtio-mmio-bus.N`**。务必用 `bus=` 显式钉槽位，避免 QEMU 逆序自动分配踩坑。

- ch1/ch3/ch4（无块设备）：GPU 占 **bus.0 = 0x1000_1000**，键盘占 bus.1 = 0x1000_2000。
- ch7/ch8（块设备占 bus.0 = 0x1000_1000）：GPU 占 **bus.1 = 0x1000_2000**，键盘占 bus.2 = 0x1000_3000。

> 所以 ch1~ch4 调研里写「GPU@0x10001000」、ch8 调研里写「GPU@0x10002000」并不矛盾——是槽位不同。
> 内核里的 `const VIRTIO_GPU: usize` 必须按本章实际槽位填。

runner 片段（在 `-kernel` **之前**插入设备；ch7/ch8 保留已有 blk 行）：
```toml
# 无 blk 的章（ch1/ch3/ch4）
"-device", "virtio-gpu-device,bus=virtio-mmio-bus.0,xres=640,yres=480",
"-device", "virtio-keyboard-device,bus=virtio-mmio-bus.1",
# 有 blk 的章（ch7/ch8），blk 已在 bus.0：
"-device", "virtio-gpu-device,bus=virtio-mmio-bus.1,xres=640,yres=480",
"-device", "virtio-keyboard-device,bus=virtio-mmio-bus.2",
```
QEMU 8.2.2 已验证支持 `virtio-gpu-device` 与 `virtio-keyboard-device`。
分辨率统一用 **640×480**（≈1.18 MiB fb，适配 ch3 静态 DMA 与 ch4+ 单段映射）。

### A.3 HAL 代码骨架（两套，按 satp 模式选）

模板：`tg-rcore-tutorial-ch8/src/virtio_block.rs:58-96`。GPU/Input **复用同一个 HAL**。

**(a) Bare 模式（ch1/ch3，satp=Bare，恒等映射，无堆）** —— 静态 DMA 池：
```rust
// 在 .bss 预留 DMA 池（无堆）：[u8; N*4096]，用静态位图分配页
static mut DMA_POOL: [u8; 2 * 1024 * 1024] = [0; 2*1024*1024]; // 2 MiB
static mut DMA_BITMAP: [u64; 8] = [0; 8];                       // 512 页位图
struct VirtioHal;
impl Hal for VirtioHal {
    fn dma_alloc(pages: usize) -> usize { /* 位图找连续 pages 页，返回 &DMA_POOL[off] 物理地址 */ }
    fn dma_dealloc(paddr: usize, pages: usize) -> i32 { /* 清位图 */ 0 }
    fn phys_to_virt(paddr: usize) -> usize { paddr } // 恒等
    fn virt_to_phys(vaddr: usize) -> usize { vaddr } // 恒等
}
```

**(b) Sv39 模式（ch4/ch7/ch8，有堆）** —— 直接照搬 ch8：
```rust
impl Hal for VirtioHal {
    fn dma_alloc(pages: usize) -> usize {                 // alloc_zeroed 连续物理页
        unsafe { alloc_zeroed(Layout::from_size_align_unchecked(
            pages << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS)) as _ } }
    fn dma_dealloc(paddr: usize, pages: usize) -> i32 { /* dealloc */ 0 }
    fn phys_to_virt(paddr: usize) -> usize { paddr }     // 内核恒等
    fn virt_to_phys(vaddr: usize) -> usize {             // 查内核页表
        const VALID: VmFlags<Sv39> = build_flags("__V");
        unsafe { KERNEL_SPACE.assume_init_ref()
            .translate(VAddr::new(vaddr), VALID).unwrap().as_ptr() as usize } }
}
```

### A.4 GPU 驱动骨架（`src/gpu.rs`，全章通用）

```rust
use virtio_drivers::{VirtIOGpu, MmioTransport, VirtIOHeader};
pub struct Gpu { inner: VirtIOGpu<VirtioHal, MmioTransport>, fb_pa: usize, w: u32, h: u32 }
impl Gpu {
    pub unsafe fn init(mmio_base: usize) -> Self {
        let t = MmioTransport::new(NonNull::new(mmio_base as *mut VirtIOHeader).unwrap()).unwrap();
        let mut g = VirtIOGpu::new(t).unwrap();
        let fb: &mut [u8] = g.setup_framebuffer().unwrap();   // 长度 = w*h*4 (BGRA8888)
        let (w, h) = g.resolution().unwrap();                 // 640,480
        let fb_pa = fb.as_ptr() as usize;                     // Bare=物理；Sv39=内核恒等虚址→物理
        Self { inner: g, fb_pa, w, h }
    }
    pub fn flush(&mut self) { self.inner.flush().unwrap(); }  // transfer_to_host_2d + resource_flush
}
```
- `setup_framebuffer() -> Result<&mut [u8]>`：返回 DMA 连续缓冲，长 `w*h*4`。
- `flush()`：把 fb 内容推到屏幕；异步，截帧前需保证已调用。
- 全局实例用 `spin::Lazy<Mutex<Gpu>>`（参考 `BLOCK_DEVICE` 写法）。

### A.5 Input 驱动骨架（`src/input.rs`）+ 键码表

```rust
pub struct Input { inner: VirtIOInput<VirtioHal, MmioTransport> }
impl Input {
    pub fn poll(&mut self) -> Option<u16> {                  // 返回按下的 evdev code
        while let Some(e) = self.inner.pop_pending_event() {
            if e.event_type == 1 /*EV_KEY*/ && e.value == 1 /*按下*/ { return Some(e.code); }
        }
        None
    }
}
```
Linux evdev 键码（统一约定，游戏内自行重映射）：
`KEY_W=17 KEY_A=30 KEY_S=31 KEY_D=32`，`KEY_I=23 J=36 K=37 L=38`，
方向键 `UP=103 DOWN=108 LEFT=105 RIGHT=106`，`SPACE=57 ENTER=28 ESC=1`。

> ch1/ch3 走**最简 UART**：`tg_sbi::console_getchar()` 非阻塞读，返回 ASCII；
> 不引入 VirtIO-Input，省一套 DMA。ch4+ 再上 VirtIO-Input。

### A.6 自定义 syscall 约定（ID 2000/2001/2002）

共享 crate：`tg-rcore-tutorial-syscall`（ch3~ch8 全用它；ch1 自建迷你分发）。
三步接线（一次性改 syscall crate，全章受益）：
1. `src/syscall.h.in` 末尾追加：
   ```
   #define __NR_framebuffer_info 2000
   #define __NR_gpu_flush        2001
   #define __NR_key_event        2002
   ```
2. `src/kernel/mod.rs`：新增 `trait GPU`（仿 IO/Process/Clock trait）+ `static GPU` 容器 + `init_gpu()`，
   在 `handle()` 的 `match id` 末尾加三分支转发。
3. `src/user.rs`：加用户态 wrapper `framebuffer_info()/gpu_flush(x,y,w,h)/key_event()`。

**ABI（寄存器约定）**
| ID | 名称 | 入参 a0.. | 返回 a0 (+a1/a2/a3) | 语义 |
|----|------|----------|---------------------|------|
| 2000 | framebuffer_info | 无 | a0=用户虚址(Sv39)或物理址(Bare)，a1=width，a2=height，a3=bpp(=32) | 查 fb 与分辨率 |
| 2001 | gpu_flush | a0=x a1=y a2=w a3=h（全屏可传 0,0,W,H） | 0 成功 / -1 失败 | 触发 VirtIO flush |
| 2002 | key_event | 无 | >0=键码，0=无键 | 非阻塞取一个按键 |

> 单返回值内核（tg-syscall 的 `handle` 只回 a0）下，2000 改为：a0 返回 fb 地址，
> 宽高 bpp 用一个 `#[repr(C)] FbInfo{w,h,bpp,_pad}` 结构经用户传入的指针填回（user.rs wrapper 封装）。

### A.7 framebuffer 像素格式

VirtIO-GPU `setup_framebuffer` 固定 **B8G8R8A8_UNORM**：每像素 4 字节，内存序 `[B, G, R, A]`，bpp=32。
像素偏移 `off = (y*width + x) * 4`。画笔助手（放用户库）：
```rust
fn put(fb:&mut[u8], w:usize, x:usize, y:usize, r:u8,g:u8,b:u8){
    let o=(y*w+x)*4; fb[o]=b; fb[o+1]=g; fb[o+2]=r; fb[o+3]=0xff; }
```

---

## B. 每内核「最小改动清单」（要改的文件 + 插桩点）

### B.1 ch1 · tangram（最裸：从零搭 trap + 无堆 + 无 VM）

最重，需先补 trap 框架。要改/新建：
- `.cargo/config.toml:10` runner：加 GPU(bus.0)+键盘(bus.1)，可加 `-m 1G`。
- `src/main.rs:rust_main`（约 L65 末尾死循环处）：插 `trap_init()` → `load_app()` → `jump_to_user()`。
- 新建 `src/trap.rs`：`trap_handler`（保存/恢复 32 GPR、读 scause、ecall 分发、sepc+=4、sret）、
  `trap_init()`（写 `stvec`）。**栈从 4 KiB 提到 ≥32 KiB**（仿 ch8）。
- 新建 `src/syscall.rs`：`syscall_handler(id,a0..a6)`，实现 2000/2001/2002。
- 新建 `src/gpu.rs`：A.3(a) 静态 DMA HAL + A.4 GPU init（VIRTIO_GPU=0x1000_1000）。
- `build.rs:LINKER_SCRIPT(L39)`：加 `.bss.dma` 段给 DMA 池（或 `.text.trap`）。
- 新建 `src/app.rs`：`load_app()/jump_to_user(entry)` 切 U-mode（Bare 下 VA=PA）。
- 用户程序 `user/tangram.rs`：2000 拿物理 fb 直接画七巧板 → 2001 flush → 2002(UART) 交互。
- 分阶段：①trap+假 syscall（fb 假刷新返回 0）→ ②接真 GPU → ③（可选）才上 Sv39。

### B.2 ch3 · snake（多道/分时，satp=Bare，无堆）

复用现成 trap/调度/syscall crate，只补 GPU。要改：
- `.cargo/config.toml:10-16`：加 GPU(bus.0)+键盘(bus.1)。
- `src/main.rs`：`impls` 模块（L223 后）加 `VirtioHal`（A.3(a) 静态 DMA）+ `impl GPU for SyscallContext`；
  `rust_main` 设备注册处（L111-115）加 `Gpu::init` + `tg_syscall::init_gpu(&SyscallContext)`。
- `Cargo.toml`：确认 `virtio-drivers="0.1.0"`。
- syscall crate（A.6 三步，**一次改全章用**）。
- `tg-rcore-tutorial-user/src/lib.rs`：加 wrapper；`src/bin/snake.rs`（新）：读键→更新→直写 fb→flush。
- `tg-rcore-tutorial-user/cases.toml`：把 snake 加进 ch3 用例。
- 帧缓冲：内核经 2000 把**物理 fb 地址**给用户；Bare 无隔离故可直访（见风险 E）。

### B.3 ch4 · tetris（Sv39 + ELF 进程，有堆）

基建最顺，重点是把 fb 映射进用户空间。要改：
- `.cargo/config.toml:10-16`：加 GPU(bus.1)/键盘(bus.2)（若 ch4 无 blk 则 bus.0/bus.1，按实际槽位）。
- 新建 `src/gpu.rs`（A.3(b) 堆 HAL）/`src/input.rs`。
- `src/process.rs:Process::new`（用户栈映射 L137-141 之后）：调 `map_framebuffer()` —
  `address_space.map_extern(VPN(0x4000_0000>>12)..VPN((0x4000_0000+fb_size)>>12), PPN(fb_ppn), build_flags("U_WRV"))`，
  **flags 必含 U 位**（否则 `translate()` 在 main.rs 拒绝用户访问）；进程结构记 `fb_vaddr`。
- `src/main.rs`：`kernel_space()`（可选预留 DMA 物理区）；`schedule()` 进循环前 `Gpu::init`+`init_gpu`；
  `impls`（L646 后）`impl GPU`：2000 返 0x4000_0000+宽高，2001 调 `VirtIOGpu::flush`，2002 读 input。
- syscall crate（A.6，若 ch3 已改则复用）。
- 用户：`user/src/bin/tetris.rs` + 画矩形/像素助手。

### B.4 ch7 · pong（Sv39 + 管道/信号，有堆）

ch7 的 `Sv39Manager` 与 ch4 结构相同（`OWNED=1<<8`、`alloc_zeroed`），fb 映射照搬 ch4。要改：
- `.cargo/config.toml`（ch7 已有 blk@bus.0）：GPU→bus.1、键盘→bus.2。
- 新建 `src/gpu.rs`/`src/input.rs`（同 ch4）。
- `src/process.rs:Process::from_elf`：加 `map_framebuffer()`（同 ch4，0x4000_0000+U 位）。
- `src/main.rs`：`impls` 加 `impl GPU` + 设备初始化；MMIO 常量按 bus.1 = 0x1000_2000。
- syscall crate 复用。
- 用户 `bin/pong.rs`：利用 ch7 已有 **pipe/fork/signal** 做双玩家进程（或单进程双拍）；
  键 W/S vs ↑/↓；共享分数经管道。

### B.5 ch8 · doom（Sv39 + 线程/锁/信号量/条件变量，有堆 + easy-fs）

并发最全，做「逻辑线程 + 渲染线程」分离。要改：
- `.cargo/config.toml:14-26`：blk@bus.0 之外加 GPU@bus.1、键盘@bus.2；
  `src/main.rs:161` MMIO 表扩成 `&[(0x1000_1000,0x1000),(0x1000_2000,0x1000),(0x1000_3000,0x1000)]`。
- 新建 `src/gpu.rs`（复用 `virtio_block.rs` 的 HAL/`build_flags`/`KERNEL_SPACE`）/`src/input.rs`。
- `src/process.rs`（`map_portal` L337-340 邻近）：加 `map_framebuffer()` → 0x4000_0000、`U_WRV`。
- `src/main.rs`：`impls` 加 `impl GPU`；syscall 分发处（L224-266）经 `tg_syscall::handle` 自动转发；L202 后 `init_gpu`。
- syscall crate 复用。
- 用户 `bin/doom.rs`：主线程逻辑+读键，子线程周期 `gpu_flush`；共享 fb 用 **Mutex**；
  关卡/贴图经 easy-fs `open/read` 加载（内核 fs 无需改）。fb size 取 640×480×4≈1.18 MiB。

---

## C. 实现顺序建议（谁先打基建）

1. **先在 syscall crate 打地基**（A.6）：`syscall.h.in` + `kernel/mod.rs` 的 `trait GPU`/`init_gpu`/dispatch +
   `user.rs` wrapper。一次成型，ch3~ch8 全部复用——**这是收益最高的第一步**。
2. **以 ch4-tetris 作参考实现**：Sv39+堆+ELF 进程齐全，fb 用户映射路径最干净，把 `gpu.rs`/`input.rs`/
   `map_framebuffer()` 跑通、截帧验证 OK，作为「黄金样板」。
3. **横向移植到 ch7-pong、ch8-doom**：三者 `Sv39Manager`/`map_extern` 同构，复制 `gpu.rs`/`input.rs`，
   只改 MMIO 槽位常量与 `map_framebuffer` 调用点；ch8 额外做渲染线程。
4. **回填 ch3-snake**：换 A.3(a) 静态 DMA HAL、UART 输入；验证「无堆 + Bare」路径。
5. **最后啃 ch1-tangram**：从零补 trap 框架最重，放最后；先做①假 GPU 通路再接真设备。

> 即：**syscall crate → ch4 样板 → ch7/ch8 复制 → ch3 降级 → ch1 兜底**。

---

## D. 无头截帧验证标准流程

环境已验证：QEMU 8.2.2、socat、python3.12、ffmpeg6.1（PPM→PNG）；无 ImageMagick/netpbm（用 ffmpeg 即可）。

1. **起 QEMU（headless + monitor socket + serial log）**：
   ```
   qemu-system-riscv64 -machine virt -bios none -kernel <ELF> \
     [ -drive file=fs.img,if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 ] \
     -device virtio-gpu-device,bus=virtio-mmio-bus.<N>,xres=640,yres=480 \
     -device virtio-keyboard-device,bus=virtio-mmio-bus.<N+1> \
     -display none \
     -monitor unix:/tmp/qemu-monitor.sock,server,nowait \
     -serial file:/tmp/qemu-serial.log
   ```
2. **等启动就绪**：轮询 `/tmp/qemu-monitor.sock` 出现（~2s）+ `grep -q "User programs\|init done"` serial log，
   再 `sleep 1` 余量。失败标志：`kernel panic`、设备超时、syscall unimplemented。
3. **注入按键**：`echo "sendkey i" | socat - unix-connect:/tmp/qemu-monitor.sock`；
   序列示例（上3右2下2左2）：`for k in i i i l l k k j j; do echo "sendkey $k"|socat - unix-connect:/tmp/qemu-monitor.sock; sleep .1; done`。
4. **截帧**：`echo "screendump /tmp/frame.ppm" | socat - unix-connect:/tmp/qemu-monitor.sock`
   （PPM 为 P6/RGB24，640×480≈900 KiB）。
5. **转 PNG 再看**：`ffmpeg -i /tmp/frame.ppm /tmp/frame.png -y` → Read 工具看 PNG（Read 不直接吃 PPM）。
   校验：`file /tmp/frame.ppm`（P6）、`head -3`（magic/宽/高）。
6. 备用：`screendump.py` / `key_inject.py`（python socket，默认键间隔 0.1s）。

---

## E. 主要风险与对策

**通用**
- **MMIO 槽位不确定**：QEMU 逆序自动分配会把 GPU/键盘地址搞错 → 一律 `bus=virtio-mmio-bus.N` 钉死，
  内核 `VIRTIO_GPU/VIRTIO_INPUT` 常量按本章实际槽位填；可 `-M virt,dumpdtb=qemu.dtb` 核对。
- **GPU flush 异步**：截帧/换帧前必须已 `flush()`；ch8 渲染线程用 fence/双缓冲，避免撕裂。
- **syscall ID 2000-2002 撞车**：改前 grep `syscall.h.in` 与生成的 `OUT_DIR/syscalls.rs` 确认 rCore 上游未占用。
- **DMA 内存**：动态 fb 进程退出要释放；静态区按 640×480×4≈1.18 MiB 预留足量。

**ch1（多裸）**
- 无 trap 框架 → 手写汇编 trap，寄存器保存/恢复易损坏上下文 → 先跑通「假 GPU」最小通路再接真设备。
- 4 KiB 栈在 trap 里溢出 → 提到 ≥32 KiB。
- 无堆 → 必须 `.bss` 静态 DMA 池 + 位图分配；勿用 `alloc_zeroed`。
- 若贸然上 Sv39 工作量爆炸 → ch1 默认 Bare（VA=PA），Sv39 列为可选扩展。

**ch3（无堆 DMA + Bare 无隔离）**
- 无堆 → 同 ch1 静态 DMA 池；固定缓冲限制分辨率/特性，按 640×480 预算。
- satp=Bare 无 U/K 隔离 → 用户可直访 GPU 寄存器（安全洞，仅教学演示接受；ch4+ Sv39+U 位修复）。
- 输入语义未定 → ch3 先用 UART（`console_getchar` 非阻塞），简单稳妥。
- 时钟 12500 cycle 可能太粗，手感差 → 可调 timer 或改中断驱动输入。

**ch4+（fb 映射用户空间）**
- **U 位缺失**：`map_framebuffer` flags 不含 U → `translate()` 拒绝用户访问、触发 PageFault →
  flags 串务必 `"U_WRV"`。
- DMA 物理页超堆 → 静默失败；必要时 `kernel_space()` 预留专用 DMA 物理区。
- fb 映射与用户 mmap/用户栈区间重叠 → 0x4000_0000 当前安全（栈在 0x3fffe000 下方），
  但需在 `mmap` 校验里把 0x4000_0000 段标为保留，禁止用户再映。
- Portal 共享页：GPU syscall handler 只传 `Caller` entity id，进程状态在 PROCESSES 表里查，勿在 portal 内引用进程指针。
- ch8 多线程共享 fb → Mutex/原子保护，防渲染线程与逻辑线程竞态。

---

## 附：关键参考锚点
- HAL/全局设备样板：`tg-rcore-tutorial-ch8/src/virtio_block.rs:58-96`（`Hal` 四方法 + `Lazy<Mutex>` 实例）。
- fb 用户映射：ch4 `src/process.rs`（`map_extern` + `build_flags("U_WRV")`）、ch8 `src/process.rs:337-340`。
- syscall 接线：`tg-rcore-tutorial-syscall/src/{syscall.h.in, kernel/mod.rs, user.rs}`。
- ch7 与 ch4 同构：`Sv39Manager`（`OWNED=1<<8`、`alloc_zeroed`），fb 映射直接照搬。
- virtio-drivers 0.1.0：`VirtIOGpu::{setup_framebuffer, flush, resolution}`、`VirtIOInput::pop_pending_event`（`InputEvent{event_type,code,value}`，EV_KEY=1）。
