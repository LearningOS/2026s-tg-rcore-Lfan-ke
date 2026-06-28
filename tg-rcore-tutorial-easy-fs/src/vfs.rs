use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }

    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }

    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }

    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.name() == name {
                return Some(dirent.inode_number());
            }
        }
        None
    }

    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        // 目录查找流程：目录 inode -> 遍历 dirent -> 定位子 inode 的磁盘位置。
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }

    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        // 先按“新增块数”批量申请数据块，再一次性扩容 inode。
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }

    /// Create inode under current inode by name.
    /// Attention: use find previously to ensure the new file not existing.
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        // 1) 分配新 inode
        let new_inode_id = fs.alloc_inode();
        // 2) 初始化 inode 元数据
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        // 3) 在当前目录追加 dirent 项
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // 4) 返回新文件的 Inode 句柄
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // release efs lock automatically by compiler
    }

    /// List inodes by id under current inode
    pub fn readdir(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }

    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }

    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }

    /// Return (inode id, hard-link count, is_dir) of this inode.
    pub fn stat(&self) -> (u32, u32, bool) {
        let fs = self.fs.lock();
        let ino = fs.get_disk_inode_id(self.block_id as u32, self.block_offset);
        self.read_disk_inode(|disk_inode| (ino, disk_inode.nlink, disk_inode.is_dir()))
    }

    /// Create a hard link `new_name` under this directory pointing at the same
    /// inode as the existing `old_name`. Returns 0 on success, -1 on failure.
    pub fn link(&self, old_name: &str, new_name: &str) -> isize {
        let mut fs = self.fs.lock();
        // locate the inode that `old_name` refers to
        let inode_id = self.read_disk_inode(|root| self.find_inode_id(old_name, root));
        let inode_id = match inode_id {
            Some(id) => id,
            None => return -1,
        };
        // append a new directory entry pointing to that inode
        self.modify_disk_inode(|root| {
            let file_count = (root.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            self.increase_size(new_size as u32, root, &mut fs);
            let dirent = DirEntry::new(new_name, inode_id);
            root.write_at(file_count * DIRENT_SZ, dirent.as_bytes(), &self.block_device);
        });
        // bump the hard-link count of the target inode
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
        get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |inode: &mut DiskInode| {
                inode.nlink += 1;
            });
        block_cache_sync_all();
        0
    }

    /// Remove the directory entry `name`. The hard-link count of the target
    /// inode is decremented; when it reaches zero the inode together with all
    /// its data blocks is reclaimed. Returns 0 on success, -1 on failure.
    pub fn unlink(&self, name: &str) -> isize {
        let mut fs = self.fs.lock();
        // remove the matching directory entry and obtain its inode id
        let inode_id = self.modify_disk_inode(|root| {
            let file_count = (root.size as usize) / DIRENT_SZ;
            let mut target: Option<usize> = None;
            let mut inode_id = 0u32;
            let mut dirent = DirEntry::empty();
            for i in 0..file_count {
                root.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device);
                if dirent.name() == name {
                    target = Some(i);
                    inode_id = dirent.inode_number();
                    break;
                }
            }
            let idx = target?;
            // compact: move the last entry into the freed slot, then shrink
            if idx != file_count - 1 {
                let mut last = DirEntry::empty();
                root.read_at(
                    (file_count - 1) * DIRENT_SZ,
                    last.as_bytes_mut(),
                    &self.block_device,
                );
                root.write_at(idx * DIRENT_SZ, last.as_bytes(), &self.block_device);
            }
            root.size -= DIRENT_SZ as u32;
            Some(inode_id)
        });
        let inode_id = match inode_id {
            Some(id) => id,
            None => return -1,
        };
        // decrement nlink; reclaim inode + data blocks when it hits zero
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
        let remaining = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |inode: &mut DiskInode| {
                inode.nlink -= 1;
                inode.nlink
            });
        if remaining == 0 {
            let blocks = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
                .lock()
                .modify(block_offset, |inode: &mut DiskInode| {
                    inode.clear_size(&self.block_device)
                });
            for data_block in blocks {
                fs.dealloc_data(data_block);
            }
            fs.dealloc_inode(inode_id);
        }
        block_cache_sync_all();
        0
    }

    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }
}
