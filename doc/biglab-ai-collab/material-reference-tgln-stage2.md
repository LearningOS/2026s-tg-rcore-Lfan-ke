---
name: material-reference-tgln-stage2
description: PRIMARY read-only reference library for all OS-course topics — look here FIRST
metadata: 
  node_type: memory
  type: reference
  originSessionId: e45711e0-5bd4-4c7e-988c-3b5f66ac3c56
---

`/home/heke/tgln/stage2/material/` is the **primary READ-ONLY reference** for every OS topic in this course. When you need reference for ANY subject (kernel forms, libc/musl, compiler-rt, fs, hal, net, gui, async, hypervisor, boot/SBI…), **look here first**. Never modify anything under `/home/heke/tgln/`. Master index: `notes/00-01-material-index.md`.

Design philosophy the user stated: understand the principle, give **basic** principle-teaching experiments (like the 不正经 track) — do NOT fully reimplement musl/llvm/etc.

**Sibling reference under `~/tgln/`:** besides `material/` (the resources folder), there's also **`~/tgln/.../biglab/`** which holds READ-ONLY **reference experiments** (existing labs) — consult it to compare/borrow experiment shapes for OSLAB. Note: many `material/core/*`, `hyper/*` etc. are git **submodules** — if a dir is empty it's an uninitialized submodule, not a missing topic.

**Subdir map:**
- `sbi/` OpenSBI/RustSBI/riscv-pk · `boot/` U-Boot/barebox/GRUB2/EDK2/OP-TEE · `rtos/` FreeRTOS/rt-thread/embassy
- `core/` kernels (18 projects, 6 paradigms): mono=xv6/tg-rcore/DragonOS/StarryOS · micro=seL4/Zircon/zCore · exo=jos · libOS/unikernel=unikraft/libos(HermitOS)/biscuit · framekernel=asterinas · hybrid=NT/XNU(notes)
- `hyper/` hypervisors axvisor/xen/bao/hypocaust/RVM1.5/crosvm/firecracker + RISC-V H-ext · `hal/` polyhal
- `fs/` VFS/ext*/FAT/littlefs/easyfs · `net/` lwip/smoltcp/DPDK · `gui/` X.Org/Wayland/wlroots/mesa/DRM-KMS
- `libc/` musl/relibc/uclibc-ng/newlib · `user/` compiler-builtins(=compiler-rt)/relibc · `async/` tokio/monoio/smol/embassy/corosensei
- `others/` eBPF/GPGPU(Vortex+POCL)/containers(runc/containerd)/test-suites(LTP/syzkaller/xfstests) · `rootfs/` buildroot/busybox · `distro/` Yocto/openRuyi

**Kernel-form → resources** (for the `forms/` track): overview `notes/04-02-os-kernel-paradigms.md`; mono `04-05`+core/xv6; micro `04-07`+core/seL4,zCore; exo `04-08`+core/jos; unikernel/libOS `04-09`+core/unikraft,libos; framekernel `04-06`+core/asterinas. See [[bsv-reference-datenlord]] for the separate BSV repo.
