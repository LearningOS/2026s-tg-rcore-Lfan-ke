# ch6 文件系统（硬链接）实现笔记

## 测试结果（证据）

```
✓ ch6 基础测试通过      (Test PASSED: 15/15)
✓ ch6 练习测试通过      (Test PASSED: 33/33)
```

练习测例 `ch6_usertest` 通过 `spawn` 串行启动全部子测例，关键断言：

```
[PASS] found <Test fstat OK!>
[PASS] found <Test link OK!>
[PASS] found <Test mass open/unlink OK!>
[PASS] found <Test 04_1 OK!> / <Test 04_5 ummap OK!> / <Test spawn0 OK!> ...
ch6 Usertests passed!
```

## 1. nlink 跟踪位置

- 直接存在磁盘 inode 里：`tg-rcore-tutorial-easy-fs/src/layout.rs` 的
  `DiskInode` 新增字段 `pub nlink: u32`（位于 `indirect2` 之后、`type_` 之前）。
- `DiskInode::initialize()` 中初始化 `nlink = 1`，因此 `create()` 出的文件天然 nlink=1。
- 代价：`DiskInode` 体积 +4 字节 → 每块 inode 数随 `size_of::<DiskInode>()` 自动变化；
  因 `efs.rs::get_disk_inode_pos`/`get_disk_inode_id` 都基于 `size_of::<DiskInode>()`
  动态计算，且 fs.img 由同一份 easy-fs 在构建期生成，布局自洽，无需手动改 direct count。

## 2. link / unlink / 目录项压缩（`easy-fs/src/vfs.rs`）

- `Inode::link(old, new)`：
  - `find_inode_id(old)` 拿到目标 inode_id；不存在→ -1。
  - 仿 `create()`：给根目录 `increase_size` 扩 1 个 `DIRENT_SZ`，`write_at` 追加
    `DirEntry::new(new, inode_id)`。
  - 目标 inode `nlink += 1`。返回 0。
  - 同名 old==new 在上层 `fs.rs::FileSystem::link` 拦截返回 -1。
- `Inode::unlink(name)`：
  - 遍历根目录定位目录项下标 `idx`；找不到→ -1。
  - **压缩**：把最后一项搬到 `idx` 槽（`idx != last` 时），再 `root.size -= DIRENT_SZ`，
    与 `find/readdir` 按 `size/DIRENT_SZ` 顺序扫描的读法一致。
  - 目标 inode `nlink -= 1`。

## 3. nlink 归零时回收 inode + 数据块（`vfs.rs` + `efs.rs`）

- `unlink` 中 `nlink` 减到 0 时：
  - `DiskInode::clear_size()` 清空并返回所有数据/索引块号 → 逐个 `fs.dealloc_data(b)`。
  - `fs.dealloc_inode(inode_id)` 释放 inode 位图位。
- `efs.rs` 新增：
  - `dealloc_inode(id)` → `inode_bitmap.dealloc`。
  - `get_disk_inode_id(block_id, offset)` → 由磁盘位置反算 inode 号（fstat 用）。
  - （`dealloc_data` 原已存在。）

## 4. fstat / Stat 布局

- `Stat`/`StatMode` 定义在 `tg-rcore-tutorial-syscall/src/fs.rs`（经 `pub use fs::*` 暴露为
  `tg_syscall::{Stat, StatMode}`，用户态与内核共享同一定义）：
  `#[repr(C)] { dev:u64, ino:u64, mode:StatMode(u32), nlink:u32, pad:[u64;7] }`。
  `pad` 私有 → 内核侧用 `Stat::new()` 取零值后再填 `dev/ino/mode/nlink` 公有字段。
- `Inode::stat() -> (u32 ino, u32 nlink, bool is_dir)`（vfs.rs）：用
  `efs.get_disk_inode_id` 求 ino，读盘取 `nlink` 与 `is_dir()`。
- 内核 `fstat`（main.rs）：取 `fd_table[fd]` → `FileHandle.inode` → `inode.stat()`，
  `translate::<Stat>(st, WRITEABLE)` 写回；mode = DIR/FILE。

## 5. 内核侧改动（`tg-rcore-tutorial-ch6/src/main.rs` 的 `impls`）

- 新增辅助 `translate_user_str(current, ptr) -> Option<String>`：翻译用户虚址后逐字节读
  到 `\0`（linkat/unlinkat 路径用）。
- `linkat`(37)：翻译 old/new 两路径 → `FS.link`；任一翻译失败或同名 → -1。
- `unlinkat`(35)：翻译 path → `FS.unlink`。
- `fstat`(80)：见上。
- `fs.rs::FileSystem` 实现 `FSManager::{link,unlink}`，委托给 `self.root.{link,unlink}`，
  并在 link 中拦截 src==dst。

## 6. 前向兼容补全（练习测例 `ch6_usertest` 依赖）

`ch6_usertest` 用 `spawn` 启动 ch3/ch4/ch5/ch6 全部子测例，故顺带实现（之前是 stub）：

- `spawn`(main.rs)：与 ch5 同构，但程序来自磁盘——`FS.open(name, RDONLY)` + `read_all`
  → `ElfFile::new` → `Process::from_elf` → `PROCESSOR.add(pid, child, parent_pid)`。
- `mmap`/`munmap`(main.rs)：移植 ch5 逻辑，基于 `current.address_space.areas` 做重叠/已映射
  校验，`map(&[],0,flags)` / `unmap`。`parse_flags` 已加入 impls 的 `use crate::{...}`。

## 涉及的跨 crate 符号

- `tg_syscall::{Stat, StatMode}`（共享）、`IO::{linkat,unlinkat,fstat}`、`Process::spawn`、
  `Memory::{mmap,munmap}`。
- `tg_easy_fs::FSManager::{link,unlink}`（trait, file.rs）、`Inode::{link,unlink,stat}`（vfs.rs）、
  `DiskInode::nlink`（layout.rs）、`EasyFileSystem::{dealloc_inode,get_disk_inode_id,dealloc_data}`（efs.rs）。

## easy-fs 改动文件清单（本地 path 依赖 `../tg-rcore-tutorial-easy-fs`，勿提交）

- `src/layout.rs`：`DiskInode.nlink` 字段 + `initialize` 置 1。
- `src/vfs.rs`：`Inode::{stat,link,unlink}`。
- `src/efs.rs`：`dealloc_inode`、`get_disk_inode_id`。
- `src/file.rs`：`FSManager` trait 增 `link/unlink`（已声明）。
