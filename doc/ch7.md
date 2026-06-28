# ch7 · 进程间通信（管道）+ 统一文件描述符 —— 实现笔记

测试结果：`./test.sh base` → **✓ 18/18**（ch7 无练习测试，`all`≡`base`）。
通过项含 `pipetest passed!` / `pipe_large_test passed!` / `file_test passed!` /
`forktest pass.` / `signal_simple: Done` / `Test sbrk almost OK!` 等。

## 实现的 TODO

### 1. `Sv39Manager::deallocate` / `drop_root`（`src/main.rs` 的 `impls` 模块）
ch7 的 `Sv39Manager` 与 ch4 结构完全相同（`struct Sv39Manager(NonNull<Pte<Sv39>>)`、
`OWNED = 1<<8`、`page_alloc` 用 `alloc_zeroed`），故逆操作照搬 ch4：
- `deallocate(pte, len)`：`p_to_v::<u8>(pte.ppn())` 取恒等映射虚地址，
  `dealloc(p, Layout(len<<PAGE_BITS, 1<<PAGE_BITS))` 释放 `len` 页，返回 `len`。
- `drop_root()`：释放根页表页 `self.0`（1 页）。
- 配套：`impls` 模块的 `use alloc::alloc::{...}` 补 `dealloc`（原本只有 `alloc_zeroed`）。
- 这两个函数在**进程退出**回收地址空间时被调用，base 测试的 `exit pass.`/`forktest` 覆盖到。

### 2. `FileSystem::{link, unlink}`（`src/fs.rs`，委托 easy-fs）
原为 `unimplemented!()`。ch7 的 `FS.root: Inode` 来自 `tg_easy_fs`，已有 `Inode::{link,unlink}`：
- `link(src, dst)`：自链接（`src==dst`）报 -1，否则 `self.root.link(src, dst)`。
- `unlink(path)`：`self.root.unlink(path)`（移除目录项、递减 nlink，归零时回收 inode+数据块）。
- 与 ch6/ch8 的 `FSManager` 实现同款委托。ch7 的 base 测试未直接调 linkat/unlinkat 系统调用，
  但按「即使指导书未列也实现全部 stub」的要求补齐，保持与 ch6/ch8 一致、跨章节可用。

## 跨 crate 符号
- `tg_easy_fs`：`FSManager{open/find/readdir/link/unlink}`、`Inode{link/unlink/find/create/...}`、
  `FileHandle`、`make_pipe`、`PipeReader/PipeWriter`、`UserBuffer`、`OpenFlags`。
- `tg_kernel_vm::PageManager{p_to_v/v_to_p/allocate/deallocate/drop_root/...}`、`page_table::{Pte/Sv39/MmuMeta/...}`。
- ch7 新增 `Fd` 枚举统一「普通文件 / 管道读端 / 管道写端 / 空(stdin/out/err)」，
  使 read/write 系统调用走同一接口（见 `fs.rs::Fd::{read,write,readable,writable}`）。
