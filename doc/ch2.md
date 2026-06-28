# ch2 · 批处理系统与特权级切换 —— 验证笔记

测试结果：`./test.sh base` → **✓ 5/5**（`✓ ch2 基础测试通过`）。

## 本章内容
在 ch1 基础上引入 **U/S 特权级隔离的批处理系统**：依次加载并运行多个用户程序，
每个程序在 U 态执行、通过 `ecall` 陷入 S 态处理系统调用（`write`/`exit`），
出错或退出后切换到下一个程序。属「Trap 机制 + 系统调用分发」章，
**无练习题、无 `todo!()`**，仓库基线即可通过。

## 关键路径
- Trap 入口保存/恢复用户上下文（`tg_kernel_context::LocalContext`），`ecall` → `UserEnvCall`。
- 系统调用经 `tg_syscall::handle` 分发到 `impls::SyscallContext` 的 `IO::write` / `Process::exit`。
- 与 ch3 的区别：ch2 是「一个接一个」批处理，尚无 TCB / 时间片轮转 / 多道并发。

## 跨 crate 符号
- `tg_syscall`：trait `IO/Process`，`Caller`，`SyscallId`，`handle`。
- `tg_kernel_context::LocalContext`：用户寄存器上下文与 `execute()`。
- `tg_linker::AppMeta`：定位内置的用户程序镜像。

> 本章无可实现的 stub；trace/计数等练习从 ch3 开始（见 [ch3 笔记](ch3.md)）。
