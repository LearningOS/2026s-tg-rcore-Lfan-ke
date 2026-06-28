# ch5 进程管理 — 实现笔记

本章在 ch4 地址空间基础上引入完整进程管理，练习实现了 spawn / set_priority / stride 调度，
并迁移了 ch4 的 mmap/munmap 与页表回收填空。改动只涉及 `tg-rcore-tutorial-ch5/src/`。

## 1. 页表回收填空（`src/main.rs` 的 `impls::Sv39Manager`）

与 ch4 同样的两处填空，使 `PageManager` trait 完整（避免 `todo!()` 在地址空间析构路径 panic）：

- `deallocate(pte, len) -> usize`：把 `pte.ppn()` 经 `p_to_v::<u8>` 转回虚拟指针，
  用 `alloc::alloc::dealloc` 释放 `len` 页（`Layout = len<<PAGE_BITS, align = 1<<PAGE_BITS`），返回 `len`。
- `drop_root()`：释放根页表所在的那 1 个物理页（`self.0`，同样的 Layout，len=1）。

注意 kernel-vm 当前没有为 `AddressSpace` 实现 `Drop`，所以这两个方法实际不会被自动调用，
但 trait 要求必须实现，且填成真实逻辑而非 `todo!()` 更安全。

## 2. mmap(222) / munmap(215) 迁移（`impls::Memory`）

迁移自 ch4，适配新的 `PROCESSOR.get_mut().current()` 取当前进程地址空间。

- **mmap**：校验顺序
  1. `addr % PAGE_SIZE == 0`（页对齐），否则 -1；
  2. `prot & !0x7 == 0 && prot & 0x7 != 0`（只能用低 3 位且非 0），否则 -1；
  3. 目标 VPN 区间 `[floor(addr), ceil(addr+len))` 不能与 `address_space.areas` 任何区间重叠，否则 -1；
  4. 由 prot 构建标志位 `U___V`（bit0=R→idx3, bit1=W→idx2, bit2=X→idx1），`parse_flags` 解析；
  5. `address_space.map(range, &[], 0, flags)`，返回 0。
- **munmap**：`addr` 必须页对齐（否则 -1）；区间内**每一页**都必须已映射
  （`areas.iter().any(|a| a.start <= vpn && vpn < a.end)`），否则 -1；然后 `address_space.unmap(range)`，返回 0。

关键 API：`AddressSpace::{map, unmap, translate, areas(pub Vec<Range<VPN>>)}`，
`VAddr::floor()/ceil()`，VPN 支持 `Ord` 和 `+ 1`。

## 3. spawn 系统调用（ID 400，`impls::Process::spawn`）

语义：直接从目标程序的 ELF **新建**一个进程，**不**复制父进程地址空间（区别于 fork+exec）。
成功返回子进程 pid，失败 -1。

实现（复用 fork 的 add 流程 + exec 的路径翻译 + from_elf 的装载）：
1. `current.address_space.translate::<u8>(VAddr::new(path), build_flags("RV"))` 把用户态路径串翻译到内核可读指针，
   `from_raw_parts(ptr, count)` 还原 `&str`；翻译失败返回 -1。
2. `APPS.get(name)` 找到内联 ELF 数据，`ElfFile::new(...)`，`Process::from_elf(elf)` 建新进程。
3. `PROCESSOR.add(child.pid, child, parent_pid)` —— 第三参指明父 pid，由 `PManager` 维护 `rel_map` 父子关系，
   这样父进程的 `wait/waitpid` 才能回收该子进程。返回 `child.pid`。

用户侧（`tg-rcore-tutorial-user`）：`spawn(path) = syscall2(SPAWN, path.as_ptr(), path.len())`；
测例 `ch5_spawn0`（批量 spawn ch5_getpid + wait 计数一致性）、`ch5_spawn1`（wait/waitpid 语义对比）。

## 4. set_priority(140) + stride 调度（本章核心）

### 数据结构（`src/process.rs::Process`）
新增两个字段：`priority: usize`（初始 16，最小 2）、`stride: usize`（初始 0）。
`from_elf` 初始化 `priority=16, stride=0`；`fork` 让子进程继承父 `priority`，`stride` 重置 0。

### set_priority（`impls::Scheduling`）
`prio >= 2` 时设 `current.priority = prio as usize` 并返回 `prio`；否则返回 -1。
（测例 `ch5_setprio` 验证 10、isize::MAX 合法返回原值，0/1/负数返回 -1。）

### stride 算法（`src/processor.rs::ProcManager` 的 `Schedule::fetch`）
- 常量 `BIG_STRIDE = 1 << 40`（`BigStride`）。取这么大是为了减小整数除法误差，
  又远小于 `usize::MAX`，即使上百万调度轮次也不会溢出反转（用 `wrapping_add` 再兜底）。
- 每次 `fetch()`：暴力扫一遍 `ready_queue`，按 `tasks[id].stride` 选**最小**者
  （相等取队首，保持 FIFO 公平）；`remove` 出队；对被选中进程
  `stride = stride.wrapping_add(BIG_STRIDE / priority.max(2))`；返回该 id。
- 数学：`pass = BIG_STRIDE / priority`。priority 越大 → pass 越小 → stride 增长越慢
  → 被选中的频率越高 → 分到的 CPU 时间与 priority 成正比。

### 抢占点（无时钟中断也能时间片轮转）
本内核没有定时器抢占；`ch5_strideN` 测例在循环里每 400 次自旋调用一次 `get_time()`，
而 `get_time/clock_gettime` 是 syscall → trap → `make_current_suspend()` 把当前进程重新入队
→ 主循环 `find_next()→fetch()` 重新挑 stride 最小者。于是 syscall 边界就是调度切换点，
stride 调度按 `get_time` 粒度生效。验证结果各进程 `count/priority` 比值基本一致（约 6.5e4）。

## 5. 跨 crate 符号速查

- `tg_task_manage`：`PManager<P, MP>`（`find_next/current/add(id,task,parent)/make_current_suspend/
  make_current_exited/wait`）；trait `Manage<T,I>{insert/delete/get_mut}`、`Schedule<I>{add/fetch}`（**stride 改的就是 fetch**）；
  `ProcId{new/from_usize/get_usize}`。本章 ProcManager 用 `tasks: BTreeMap<ProcId,Process>` + `ready_queue: VecDeque<ProcId>`。
- `tg_kernel_vm`：`AddressSpace{map/unmap/map_extern/translate/cloneself/areas/root/root_ppn}`；
  `PageManager{p_to_v/v_to_p/allocate/deallocate/drop_root/...}`；`page_table::{VAddr/VPN/PPN/VmFlags/Pte/MmuMeta/Sv39}`。
- `tg_syscall`：内核侧 trait `Process/Scheduling/Memory/IO/Clock` + `init_*` 注册 + `handle(caller,id,args)`；
  `SyscallId::{SPAWN=400, SETPRIORITY=140, WAIT4, ...}`；用户侧 `spawn/set_priority/wait/waitpid`。
  `wait` 在子进程仍运行时返回 -2，用户 `wait/waitpid` 据此 `sched_yield` 自旋。
- 本 crate helper：`build_flags`(const) / `parse_flags`(运行期) 构造 `VmFlags<Sv39>`。

## 6. 通过证据

```
✓ ch5 基础测试通过      （Test PASSED: 14/14）
✓ ch5 练习测试通过      （Test PASSED: 17/17）
```

练习侧 stride 实测（一次运行）：
```
priority = 5,  ratio = 65680
priority = 6,  ratio = 65666
priority = 7,  ratio = 65714
priority = 8,  ratio = 65750
priority = 9,  ratio = 65777
priority = 10, ratio = 65840
```
各 priority 的 `count/priority` 比值基本相等，确认 CPU 时间与优先级成正比。

## 引申

- 当前 `fetch` 是 O(n) 暴力扫描；进程多时可换小根堆（`BinaryHeap` + `Reverse`）按 stride 排序，
  但需处理 stride 更新后的重新入堆，教学测例量小，暴力足够且更直观。
- 溢出严谨做法：经典实现用较小位宽 stride，保证 `max-min <= BIG_STRIDE`，比较时用 wrapping 差值判号；
  这里用 64 位 usize + 适中 BIG_STRIDE 规避，`wrapping_add` 兜底。
- spawn 目前不支持向子进程传参（argv）；可扩展为把用户栈预置 argc/argv 后再设 sp。
