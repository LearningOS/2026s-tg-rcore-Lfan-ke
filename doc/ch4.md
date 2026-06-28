# ch4 · 地址空间（Sv39 虚拟内存）—— 实现笔记

测试结果：`./test.sh base` → ✓ 6/6；`./test.sh exercise` → ✓ 16/16。

## 实现的 TODO（均在 `src/main.rs` 的 `impls` 模块 + `src/process.rs`）

### 1. `Sv39Manager::deallocate` / `drop_root`（`impls::Sv39Manager`）
`allocate` 用 `alloc_zeroed` 分配并打 `OWNED` 标志，故其逆操作：
- `deallocate(pte, len)`：`p_to_v::<u8>(pte.ppn())` 取虚地址（恒等映射），`alloc::alloc::dealloc` 释放 `len` 页，返回 `len`。
- `drop_root()`：释放根页表页 `self.0`（1 页）。

### 2. `trace` 重写（`impls::Trace`，对应 user lib `trace_read/trace_write/count_syscall`）
- `trace(0, va, _)`：读用户虚地址 `va` 处一字节。`address_space.translate::<u8>(va, READABLE)`，`READABLE = build_flags("U_RV")`，返回值或 -1。
- `trace(1, va, data)`：写。`WRITABLE = build_flags("U_WV")`，写入 `data as u8`，返回 0 或 -1。
- **`trace(2, syscall_id, _)`：返回该进程调用 `syscall_id` 的次数**（`process.syscall_count[syscall_id]`）。
- **关键坑**：READABLE/WRITABLE 必须含 **U** 位。`isize::MAX` 掩码后撞上 portal 页（VPN::MAX）、`0x80200000` 是内核恒等映射页，它们有 R+V 但无 U；缺 U 检查会误判用户"可读/可写"。

### 3. 系统调用计数（`src/main.rs` 调度循环 + `src/process.rs`）
- `Process` 加字段 `syscall_count: Box<[u32; MAX_SYSCALL=512]>`。**必须 `Box`**：内联 `[u32;512]`(2KB) 会让 `Process` 过大，`new()` 在栈上构造再移动时溢出内核栈 → 表现为大量测试莫名失败。
- 调度循环 `UserEnvCall` 分支：取 `syscall_id = ctx.a(7)`，在调 `tg_syscall::handle` **之前**自增 `PROCESSES[0].syscall_count[syscall_id]`。**计数含本次调用**（ch3_trace 注释「这次 trace 调用本身也计入」，验算 SYS_TRACE 序列 2→7 吻合）。

### 4. `mmap`(222) / `munmap`(215)（`impls::Memory`，见 `exercise.md`）
- `mmap`：校验 `addr` 页对齐、`prot & !0x7 == 0`、`prot & 0x7 != 0`、`[addr,addr+len)` 与 `address_space.areas` 不重叠；由 prot 位构造 flags（`build_flags("U_V")` 基础上按位 `|=` R/W/X）；`address_space.map(range, &[], 0, flags)`（匿名=空数据）。返回 0/-1。
- `munmap`：`addr` 页对齐 + 范围内每页都已映射（否则 -1），`address_space.unmap(range)`。

## 跨 crate 符号
- `tg_kernel_vm`：`AddressSpace{map/unmap/translate/areas/root_ppn}`、`PageManager{p_to_v/v_to_p/allocate/deallocate/drop_root}`、`page_table::{VAddr/VPN/PPN/VmFlags/Pte/Sv39/MmuMeta}`。
- `tg_syscall`：trait `IO/Process/Memory/Scheduling/Clock/Trace`，`Caller`，`SyscallId`。
- `tg_kernel_context`：`ForeignContext/LocalContext`、`MultislotPortal`。
- 标志字符串走 `build_flags`(const) / `parse_flags`(运行时)；运行时可 `build_flags("U_V") | build_flags("R")` 组合。

---

## 复测验证（2026-06-27）

独立复测当前磁盘上的 ch4 源码（`src/main.rs` 的 `impls` + `src/process.rs`），与上文方案一致：

```
✓ ch4 基础测试通过      # Test PASSED: 6/6
✓ ch4 练习测试通过      # Test PASSED: 16/16
```

exercise 关键输出：`get_time OK!` / `Test sleep OK!` / `Test sleep1 passed!` /
`string from task trace test` / `Test trace OK!` / `Test 04_1 OK!` / `Test 04_4 test OK!` /
`Test 04_5 ummap OK!` / `Test 04_6 ummap2 OK!` / `Test trace_1 OK!`。

踩坑复盘（与上文呼应，附实测现象）：
- **缺 U 位**：`trace_read(isize::MAX)` 经 `VAddr::floor` 截断撞上 `VPN::MAX`（传送门页 `__G_XWRV`，无 U）。
  用不含 U 的 `RV` 检查会误返回 `Some` → `ch4_trace.rs:23 assert_eq!(None, ...)` 失败。改用 `U_RV`/`U_WV` 修复。
- **未装箱**：`[u32;512]` 内联进 `Process` 后，退出时 `Vec::remove(0)` 在 8KB 调度栈上 memcpy 整个 `Process`，
  触发 `StorePageFault`（stval≈`0x3fffffdff8`，越过调度栈下界）→ 调度线程崩在 `main.rs:209`，几乎所有用例无输出。
  改成 `Box<[u32; MAX_SYSCALL]>` 后 `Process` 仅多一个指针，问题消失。

