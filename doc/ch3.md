# ch3 · 多道程序与分时多任务 —— 实现笔记

测试结果：`./test.sh base` → **✓ 4/4**；`./test.sh exercise` → **✓ 7/7**。
exercise 关键通过项：`Test sleep OK!` / `Test sleep1 passed!` /
`string from task trace test` / `Test trace OK!`。

## 实现的练习：`trace` 系统调用（号 410）

原 `impls::SyscallContext::trace` 是占位实现（打印 "trace: not implemented" 返回 -1）。
测试 `ch3_trace.rs` 验证三种请求 + 系统调用计数语义。

### 三种请求（`trace(request, id, data)`）
- **request 0 读用户内存**：返回 `id` 处一字节（0..=255），失败 -1。
  本章 `satp=Bare`（**恒等映射，无 Sv39 分页**），S 态可直接访问用户内存，
  故 `(id as *const u8).read_volatile() as isize` 即可（无需页表翻译，区别于 ch4）。
- **request 1 写用户内存**：`(id as *mut u8).write_volatile(data as u8)`，返回 0。
- **request 2 查询调用计数**：返回**当前任务**对调用号 `id` 的累计次数。

### 计数表的位置与隔离（关键设计）
- **不放进 `TaskControlBlock`，放全局 `.bss`**：`APP_CAPACITY=32`，内核栈仅
  `(32+2)*8 KiB = 272 KiB`，而 `tcbs: [TaskControlBlock; 32]` 已在栈上占 ~256 KiB
  （每个 TCB 含 `stack: [usize;1024]`=8 KiB）。再把 `[u32;512]` 内联进 TCB 会**溢出内核栈**。
  且 ch3 **无堆**（不能像 ch4 那样 `Box`）。
  → 用 `static mut SYSCALL_COUNT: [[u32; MAX_SYSCALL]; APP_CAPACITY]`（二维：任务下标 × 调用号）。
- **按任务下标隔离**：时钟抢占让多任务交替执行，若用单一全局计数会被别的任务污染
  （`assert_eq!(0, count_syscall(SYS_WRITE))` 会因别的任务的 write 而失败）。
  任务下标经 `Caller.entity` 传入：`handle_syscall(idx)` 里 `Caller { entity: idx, .. }`，
  trace 请求 2 用 `caller.entity` 索引。

### 计数时机：**调用前自增（含本次）**
`handle_syscall` 在分发 `tg_syscall::handle` **之前**自增 `SYSCALL_COUNT[idx][id]`。
测试要求 `count_syscall(SYS_TRACE)` 在第 2 次 trace 调用时返回 2、第 7 次时返回 7
（注释「这次 trace 调用本身也计入」）——只有「调用前自增」才能让当前 trace 调用被计入。

### deny(warnings) 注意
`#![cfg_attr(target_arch="riscv64", deny(warnings, missing_docs))]`：
- 访问 `static mut` 用 `&raw mut` / `&raw const` 取裸指针再解引用，避免 `static_mut_refs` 警告。
- `SYSCALL_COUNT`/`MAX_SYSCALL` 为私有项（非 `pub`），不触发 `missing_docs`（仍补了文档）。

## 改动文件
- `src/main.rs`：新增 `MAX_SYSCALL`/`SYSCALL_COUNT`；重写 `impls` 的 `Trace::trace`；
  调度循环 `tcb.handle_syscall(i)` 传入任务下标。
- `src/task.rs`：`handle_syscall(&mut self, idx)` 增参；分发前自增计数；`Caller.entity = idx`。

## 跨 crate 符号
- `tg_syscall`：trait `Trace`，`Caller{entity,flow}`，`SyscallId(pub usize)`，`handle/init_trace`。
- `tg_kernel_context::LocalContext`（TCB 的 `ctx`，`a(7)` 取调用号、`a(0..5)` 取参数）。
