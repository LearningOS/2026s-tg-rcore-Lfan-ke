//! 进程与线程管理模块
//!
//! ## 与第七章的区别
//!
//! 第七章中 `Process` 既是资源容器又是执行单元。
//! 第八章将两者分离：
//! - **Process**：资源容器，管理地址空间、文件描述符、**同步原语列表**、信号
//! - **Thread**：执行单元，管理 TID 和上下文
//!
//! 同一进程的所有线程共享 `Process` 中的资源。
//!
//! ## 新增字段
//!
//! | 字段 | 说明 |
//! |------|------|
//! | `semaphore_list` | 信号量列表（进程内所有线程共享） |
//! | `mutex_list` | 互斥锁列表 |
//! | `condvar_list` | 条件变量列表 |
//!
//! 教程阅读建议：
//!
//! - 先看 `Process` 与 `Thread` 的字段分工：明确“资源归进程、执行归线程”；
//! - 再看 `fork/exec/from_elf`：理解跨线程模型后，进程复制与替换语义如何变化；
//! - 最后结合 `processor.rs` 看线程生命周期与进程资源回收的关系。

use crate::{
    build_flags, fs::Fd, map_portal, parse_flags, processor::ProcessorInner, Sv39, Sv39Manager,
    PROCESSOR,
};
use alloc::{alloc::alloc_zeroed, boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use core::alloc::Layout;
use spin::Mutex;
use tg_kernel_context::{foreign::ForeignContext, LocalContext};
use tg_kernel_vm::{
    page_table::{MmuMeta, VAddr, PPN, VPN},
    AddressSpace,
};
use tg_signal::Signal;
use tg_signal_impl::SignalImpl;
use tg_sync::{Condvar, Mutex as MutexTrait, Semaphore};
use tg_task_manage::{ProcId, ThreadId};
use xmas_elf::{
    header::{self, HeaderPt2, Machine},
    program, ElfFile,
};

/// 线程（执行单元）
///
/// 每个线程有独立的 TID 和上下文（寄存器状态、satp）。
/// 同一进程的多个线程共享地址空间。
pub struct Thread {
    /// 线程 ID（不可变）
    pub tid: ThreadId,
    /// 执行上下文（包含 LocalContext + satp）
    pub context: ForeignContext,
}

impl Thread {
    /// 创建新线程
    pub fn new(satp: usize, context: LocalContext) -> Self {
        Self {
            tid: ThreadId::new(),
            context: ForeignContext { context, satp },
        }
    }
}

/// 死锁检测状态（**本章练习新增**）
///
/// 基于银行家算法，为 mutex 与 semaphore **分别**维护三组矩阵：
/// - `available[res]`：第 `res` 类资源当前可用（空闲）数量；
/// - `allocation[tid][res]`：线程 `tid` 当前已持有第 `res` 类资源的数量；
/// - `need[tid][res]`：线程 `tid` 当前还在等待的第 `res` 类资源数量。
///
/// 线程行（`allocation`/`need`）以全局 `ThreadId` 的整数值为键，资源以
/// `mutex_list`/`semaphore_list` 中的下标为列。开启检测后，`mutex_lock` 与
/// `semaphore_down` 在真正获取资源前先把请求加入 `need` 并运行安全性检查，
/// 若系统进入不安全状态（可能死锁）则撤销请求并返回 `-0xDEAD`。
#[derive(Default)]
pub struct DeadlockDetect {
    /// 是否为当前进程启用死锁检测
    pub enabled: bool,
    /// mutex 可用向量
    pub mutex_available: Vec<isize>,
    /// mutex 分配矩阵（tid -> 各 mutex 持有数）
    pub mutex_allocation: BTreeMap<usize, Vec<isize>>,
    /// mutex 需求矩阵（tid -> 各 mutex 待获取数）
    pub mutex_need: BTreeMap<usize, Vec<isize>>,
    /// semaphore 可用向量
    pub sem_available: Vec<isize>,
    /// semaphore 分配矩阵（tid -> 各 semaphore 持有数）
    pub sem_allocation: BTreeMap<usize, Vec<isize>>,
    /// semaphore 需求矩阵（tid -> 各 semaphore 待获取数）
    pub sem_need: BTreeMap<usize, Vec<isize>>,
}

/// 把一行向量扩展到至少 `len` 列（补 0）
fn grow_row(row: &mut Vec<isize>, len: usize) {
    while row.len() < len {
        row.push(0);
    }
}

/// 银行家安全性算法：给定 `available`、`allocation`、`need`，
/// 判断是否存在一种线程执行顺序能让所有线程都顺利结束。
fn is_safe(
    available: &[isize],
    allocation: &BTreeMap<usize, Vec<isize>>,
    need: &BTreeMap<usize, Vec<isize>>,
) -> bool {
    let m = available.len();
    let mut work: Vec<isize> = available.to_vec();
    // 参与检测的线程 = 出现在 allocation 中的所有线程
    let tids: Vec<usize> = allocation.keys().cloned().collect();
    let mut finish: BTreeMap<usize, bool> = tids.iter().map(|t| (*t, false)).collect();
    loop {
        let mut progressed = false;
        for &t in &tids {
            if finish[&t] {
                continue;
            }
            let nd = need.get(&t);
            let runnable = (0..m).all(|j| {
                let need_tj = nd.and_then(|v| v.get(j)).copied().unwrap_or(0);
                need_tj <= work[j]
            });
            if runnable {
                if let Some(al) = allocation.get(&t) {
                    for j in 0..m {
                        if let Some(v) = al.get(j) {
                            work[j] += v;
                        }
                    }
                }
                finish.insert(t, true);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    finish.values().all(|&f| f)
}

impl DeadlockDetect {
    /// 设置某类 mutex 资源（容量恒为 1）并清空相关行，列下标为 `id`
    pub fn mutex_set(&mut self, id: usize) {
        while self.mutex_available.len() <= id {
            self.mutex_available.push(0);
        }
        self.mutex_available[id] = 1;
        for row in self.mutex_allocation.values_mut() {
            grow_row(row, id + 1);
            row[id] = 0;
        }
        for row in self.mutex_need.values_mut() {
            grow_row(row, id + 1);
            row[id] = 0;
        }
    }

    /// 设置某类 semaphore 资源（容量为 `count`）并清空相关行，列下标为 `id`
    pub fn sem_set(&mut self, id: usize, count: usize) {
        while self.sem_available.len() <= id {
            self.sem_available.push(0);
        }
        self.sem_available[id] = count as isize;
        for row in self.sem_allocation.values_mut() {
            grow_row(row, id + 1);
            row[id] = 0;
        }
        for row in self.sem_need.values_mut() {
            grow_row(row, id + 1);
            row[id] = 0;
        }
    }

    /// 确保线程 `tid` 在 mutex 矩阵中存在行
    fn mutex_thread(&mut self, tid: usize) {
        let m = self.mutex_available.len();
        grow_row(self.mutex_allocation.entry(tid).or_default(), m);
        grow_row(self.mutex_need.entry(tid).or_default(), m);
    }

    /// 确保线程 `tid` 在 semaphore 矩阵中存在行
    fn sem_thread(&mut self, tid: usize) {
        let m = self.sem_available.len();
        grow_row(self.sem_allocation.entry(tid).or_default(), m);
        grow_row(self.sem_need.entry(tid).or_default(), m);
    }

    /// mutex 加锁请求安全性检查：安全返回 `true`（请求已记入 need），
    /// 不安全返回 `false`（请求已撤销，调用方应返回 -0xDEAD）
    pub fn mutex_request(&mut self, tid: usize, id: usize) -> bool {
        self.mutex_thread(tid);
        self.mutex_need.get_mut(&tid).unwrap()[id] += 1;
        if is_safe(&self.mutex_available, &self.mutex_allocation, &self.mutex_need) {
            true
        } else {
            self.mutex_need.get_mut(&tid).unwrap()[id] -= 1;
            false
        }
    }

    /// mutex 成功获取：need-1、allocation+1、available-1
    pub fn mutex_grant(&mut self, tid: usize, id: usize) {
        self.mutex_thread(tid);
        self.mutex_need.get_mut(&tid).unwrap()[id] -= 1;
        self.mutex_allocation.get_mut(&tid).unwrap()[id] += 1;
        self.mutex_available[id] -= 1;
    }

    /// mutex 释放：allocation-1、available+1
    pub fn mutex_release(&mut self, tid: usize, id: usize) {
        self.mutex_thread(tid);
        self.mutex_allocation.get_mut(&tid).unwrap()[id] -= 1;
        self.mutex_available[id] += 1;
    }

    /// semaphore 获取请求安全性检查（同 mutex_request）
    pub fn sem_request(&mut self, tid: usize, id: usize) -> bool {
        self.sem_thread(tid);
        self.sem_need.get_mut(&tid).unwrap()[id] += 1;
        if is_safe(&self.sem_available, &self.sem_allocation, &self.sem_need) {
            true
        } else {
            self.sem_need.get_mut(&tid).unwrap()[id] -= 1;
            false
        }
    }

    /// semaphore 成功获取：need-1、allocation+1、available-1
    pub fn sem_grant(&mut self, tid: usize, id: usize) {
        self.sem_thread(tid);
        self.sem_need.get_mut(&tid).unwrap()[id] -= 1;
        self.sem_allocation.get_mut(&tid).unwrap()[id] += 1;
        self.sem_available[id] -= 1;
    }

    /// semaphore 释放：allocation-1、available+1
    pub fn sem_release(&mut self, tid: usize, id: usize) {
        self.sem_thread(tid);
        self.sem_allocation.get_mut(&tid).unwrap()[id] -= 1;
        self.sem_available[id] += 1;
    }
}

/// 进程（资源容器）
///
/// 管理地址空间、文件描述符、同步原语、信号等共享资源。
/// 一个进程可以包含多个线程。
pub struct Process {
    /// 进程 ID
    pub pid: ProcId,
    /// 地址空间（所有线程共享）
    pub address_space: AddressSpace<Sv39, Sv39Manager>,
    /// 文件描述符表（所有线程共享）
    pub fd_table: Vec<Option<Mutex<Fd>>>,
    /// 信号处理器
    pub signal: Box<dyn Signal>,
    /// 信号量列表（**本章新增**，所有线程共享）
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// 互斥锁列表（**本章新增**，所有线程共享）
    pub mutex_list: Vec<Option<Arc<dyn MutexTrait>>>,
    /// 条件变量列表（**本章新增**，所有线程共享）
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
    /// 死锁检测状态（**本章练习新增**）
    pub deadlock: DeadlockDetect,
}

impl Process {
    /// exec：替换当前进程的地址空间和主线程上下文
    ///
    /// 注意：只支持单线程进程执行 exec
    pub fn exec(&mut self, elf: ElfFile) {
        let (proc, thread) = Process::from_elf(elf).unwrap();
        self.address_space = proc.address_space;
        let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
        unsafe {
            let pthreads = (*processor).get_thread(self.pid).unwrap();
            (*processor).get_task(pthreads[0]).unwrap().context = thread.context;
        }
    }

    /// fork：创建子进程（复制地址空间和主线程上下文）
    ///
    /// 子进程继承父进程的地址空间（深拷贝）、文件描述符和信号配置。
    /// 同步原语列表不继承（子进程创建空的列表）。
    pub fn fork(&mut self) -> Option<(Self, Thread)> {
        let pid = ProcId::new();
        // 深拷贝地址空间
        let parent_addr_space = &self.address_space;
        let mut address_space: AddressSpace<Sv39, Sv39Manager> = AddressSpace::new();
        parent_addr_space.cloneself(&mut address_space);
        map_portal(&address_space);
        // 复制主线程上下文
        let processor: *mut ProcessorInner = PROCESSOR.get_mut() as *mut ProcessorInner;
        let pthreads = unsafe { (*processor).get_thread(self.pid).unwrap() };
        let context = unsafe {
            (*processor).get_task(pthreads[0]).unwrap().context.context.clone()
        };
        let satp = (8 << 60) | address_space.root_ppn().val();
        let thread = Thread::new(satp, context);
        // 复制文件描述符表
        let new_fd_table: Vec<Option<Mutex<Fd>>> = self.fd_table
            .iter()
            .map(|fd| fd.as_ref().map(|f| Mutex::new(f.lock().clone())))
            .collect();
        Some((
            Self {
                pid,
                address_space,
                fd_table: new_fd_table,
                signal: self.signal.from_fork(),
                // 子进程的同步原语列表初始为空
                semaphore_list: Vec::new(),
                mutex_list: Vec::new(),
                condvar_list: Vec::new(),
                deadlock: DeadlockDetect::default(),
            },
            thread,
        ))
    }

    /// 从 ELF 文件创建进程和主线程
    ///
    /// 解析 ELF 段，建立地址空间，分配用户栈，创建初始上下文。
    pub fn from_elf(elf: ElfFile) -> Option<(Self, Thread)> {
        let entry = match elf.header.pt2 {
            HeaderPt2::Header64(pt2)
                if pt2.type_.as_type() == header::Type::Executable
                    && pt2.machine.as_machine() == Machine::RISC_V =>
            { pt2.entry_point as usize }
            _ => None?,
        };

        const PAGE_SIZE: usize = 1 << Sv39::PAGE_BITS;
        const PAGE_MASK: usize = PAGE_SIZE - 1;

        let mut address_space = AddressSpace::new();
        for program in elf.program_iter() {
            if !matches!(program.get_type(), Ok(program::Type::Load)) { continue; }
            let off_file = program.offset() as usize;
            let len_file = program.file_size() as usize;
            let off_mem = program.virtual_addr() as usize;
            let end_mem = off_mem + program.mem_size() as usize;
            assert_eq!(off_file & PAGE_MASK, off_mem & PAGE_MASK);
            let mut flags: [u8; 5] = *b"U___V";
            if program.flags().is_execute() { flags[1] = b'X'; }
            if program.flags().is_write() { flags[2] = b'W'; }
            if program.flags().is_read() { flags[3] = b'R'; }
            address_space.map(
                VAddr::new(off_mem).floor()..VAddr::new(end_mem).ceil(),
                &elf.input[off_file..][..len_file],
                off_mem & PAGE_MASK,
                parse_flags(unsafe { core::str::from_utf8_unchecked(&flags) }).unwrap(),
            );
        }
        // 分配 2 页用户栈
        let stack = unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                2 << Sv39::PAGE_BITS, 1 << Sv39::PAGE_BITS,
            ))
        };
        address_space.map_extern(
            VPN::new((1 << 26) - 2)..VPN::new(1 << 26),
            PPN::new(stack as usize >> Sv39::PAGE_BITS),
            build_flags("U_WRV"),
        );
        map_portal(&address_space);
        let satp = (8 << 60) | address_space.root_ppn().val();
        let mut context = LocalContext::user(entry);
        *context.sp_mut() = 1 << 38;
        let thread = Thread::new(satp, context);

        Some((
            Self {
                pid: ProcId::new(),
                address_space,
                fd_table: vec![
                    // stdin
                    Some(Mutex::new(Fd::Empty { read: true, write: false })),
                    // stdout
                    Some(Mutex::new(Fd::Empty { read: false, write: true })),
                    // stderr
                    Some(Mutex::new(Fd::Empty { read: false, write: true })),
                ],
                signal: Box::new(SignalImpl::new()),
                semaphore_list: Vec::new(),
                mutex_list: Vec::new(),
                condvar_list: Vec::new(),
                deadlock: DeadlockDetect::default(),
            },
            thread,
        ))
    }

    /// 把 VirtIO-GPU 帧缓冲映射进本进程用户空间固定虚址 `0x4000_0000`。
    ///
    /// `paddr` 为帧缓冲物理地址（页对齐），`len` 为字节长度。映射标志含 `U`
    /// 位，否则用户访问帧缓冲会触发 PageFault。该区间位于 ELF/堆（低地址）与
    /// 用户栈（`1<<38` 附近）之间，互不重叠。
    ///
    /// 第八章帧缓冲属于进程、为所有线程共享：DOOM 的渲染线程据此写像素，
    /// 逻辑线程不触碰它。由 `framebuffer_info` 系统调用首次触发，只映射一次。
    #[cfg(feature = "game")]
    pub fn map_framebuffer(&mut self, paddr: usize, len: usize) {
        const FB_BASE: usize = 0x4000_0000;
        const PAGE_SIZE: usize = 1 << Sv39::PAGE_BITS;
        let pages = (len + PAGE_SIZE - 1) >> Sv39::PAGE_BITS;
        let vpn_start = VAddr::<Sv39>::new(FB_BASE).floor();
        self.address_space.map_extern(
            vpn_start..vpn_start + pages,
            PPN::new(paddr >> Sv39::PAGE_BITS),
            build_flags("U_WRV"),
        );
    }
}
