# ch1 · 裸机最小内核（RV64 + SBI）—— 验证笔记

测试结果：`./test.sh base` → **✓**（`Test PASSED: Found 'Hello, world!' in output`）。

## 本章内容
最小可启动内核：关闭 std / 自定义入口、用 `tg_sbi` 通过 SBI 在 S 态打印到串口、
`#[panic_handler]` 兜底，最终 `Hello, world!`。属「环境搭建 + 启动流程」章，
**无练习题、无 `todo!()`**，仓库基线即可通过。

## 跨 crate 符号
- `tg_console`：`print!/println!` 宏与 `Console` trait（输出经 SBI 串口）。
- `tg_sbi`：`shutdown` 等 SBI 调用封装。
- `tg_linker`：内核布局/入口（链接脚本与 `KernelLayout`）。

> 本章无可实现的 stub；仅作为后续各章（ch2 批处理、ch3 多道、ch4 地址空间 …）的启动基座。
