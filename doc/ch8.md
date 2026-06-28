# ch8 并发 — 死锁检测（练习）

## 实现概览

### (A) 页管理器填空（`src/main.rs` impls 模块，与 ch4/ch5 一致）

- `Sv39Manager::deallocate(pte, len)`：
  `alloc::alloc::dealloc(self.p_to_v::<u8>(pte.ppn()).as_ptr(), Layout::from_size_align_unchecked(len << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS))`，返回 `len`。
- `Sv39Manager::drop_root()`：对根页表所在物理页 `self.0`（1 页）做 `dealloc`。

### (B) 死锁检测（核心，syscall ID 469 `enable_deadlock_detect`）

**银行家算法状态**（新增 `struct DeadlockDetect`，挂在 `Process.deadlock` 上，进程内所有线程共享，`src/process.rs`）：

- 对 **mutex** 与 **semaphore 分别** 维护三组矩阵：
  - `available[res]`：第 res 类资源当前空闲数（mutex 容量恒为 1，semaphore 容量 = `res_count`）。
  - `allocation[tid][res]`：线程 tid 当前持有数。
  - `need[tid][res]`：线程 tid 当前待获取数（pending 请求）。
- 行以全局 `ThreadId` 的整数值为键（`BTreeMap<usize, Vec<isize>>`），列以 `mutex_list`/`semaphore_list` 的下标为索引。
- `enabled: bool` 为每进程开关。

**矩阵更新点**（`src/main.rs` impls 的 `SyncMutex` 实现）：

- `mutex_create` / `semaphore_create`：`mutex_set(id)` / `sem_set(id, count)` —— 登记一类资源、`available` 置容量、各行补 0。
- `mutex_lock` / `semaphore_down`（请求点，**检查 hook**）：
  1. 若 `enabled`，先 `need[tid][res] += 1`，跑安全性检查；
  2. 不安全 → 撤销 `need[tid][res] -= 1`，**返回 `-0xDEAD`**（`-0xDEAD = -57005`，不是 -1，主循环不会阻塞它，直接把返回值写回 a0 让用户态拿到）；
  3. 安全 → 调用底层原语：立即获取成功则 `need-1 / allocation+1 / available-1`（grant）；阻塞则保留 `need`，待转交时再记账。
- `mutex_unlock` / `semaphore_up`（释放点）：释放者 `allocation-1 / available+1`；若底层返回了被唤醒线程（锁/信号量转交），对该线程 `allocation+1 / need-1 / available-1`（grant）。这样 handoff（不清 locked / count 仍为负）下账目仍满足不变式 `available = total - Σ allocation`。

**安全性检查 `is_safe`**：`work = available.clone()`；`finish[t]=false`；反复寻找 `finish[t]==false 且 need[t] <= work` 的线程，找到则 `work += allocation[t]`、`finish[t]=true`；无法推进时停止；全部 `finish` 即安全，否则不安全（可能死锁）。

**`-0xDEAD` 契约**：检测到不安全（可能死锁）时，`mutex_lock`/`semaphore_down` 立即返回 `-0xDEAD` 而非阻塞/授予；用户态据此放弃请求（测例里回滚已占资源并 `exit(-1)`）。`enable_deadlock_detect(1)` 置位返回 0，`0` 清位返回 0，其它参数返回 -1。

**关键关系**：
- 用户态自重入锁（mutex1）：单线程二次 lock，`available=0`、`need=1`，无人能推进 → 不安全 → `-0xDEAD`。
- 信号量安全场景（sem2）：阻塞但安全（某顺序可完成），放行阻塞，不误报，`failed==0`。

## 跨 crate 符号

- `tg-rcore-tutorial-sync`：`Mutex`(trait `lock(tid)->bool` / `unlock()->Option<ThreadId>`)、`MutexBlocking`、`Semaphore`(`down(tid)->bool` / `up()->Option<ThreadId>`)、`Condvar`。
- `tg-rcore-tutorial-task-manage`：`PThreadManager`（`current()`/`get_current_proc()`/`re_enque(tid)`）、`ProcId`、`ThreadId`（全局单调，`get_usize()`）。
- `tg-kernel-vm`：`PageManager` trait（`deallocate`/`drop_root`/`p_to_v`）、`AddressSpace::translate`。

## 验证证据

```
✓ ch8 基础测试通过   （Test PASSED: 22/22）
✓ ch8 练习测试通过   （Test PASSED: 25/25）
```

练习侧三条死锁测例均通过：
`deadlock test mutex 1 OK!` / `deadlock test semaphore 1 OK!` / `deadlock test semaphore 2 OK!`。
