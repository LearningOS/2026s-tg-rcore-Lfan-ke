---
name: ysyx-reference-am-nemu
description: "Local READ-ONLY YSYX (一生一芯) repos — AM/NEMU/nanos-lite, OS-teaching strengths to borrow"
metadata:
  node_type: memory
  type: reference
  originSessionId: e45711e0-5bd4-4c7e-988c-3b5f66ac3c56
---

`~/ysyx/` holds the user's YSYX projects — a famous open chip+OS education program. READ ONLY (reference only, never modify).

Key components to borrow OS-teaching strengths from (build on OSLAB's bare-metal RISC-V qemu-kernel + software-model foundation):
- **NEMU** (NJU EMUlator): build-your-own ISA emulator (instruction decode/execute, memory, devices) + **DiffTest** (差分对拍：与 Spike/QEMU 逐指令比对验证). → enriches `proper/S18-tcg` with an interpreter-vs-translator angle + a powerful verification technique.
- **AM (Abstract Machine / abstract-machine)**: clean HAL layering — **TRM**(基本计算/内存) / **IOE**(I/O) / **CTE**(上下文/trap) / **VME**(虚拟内存) / **MPE**(多处理器). → a beautiful HAL-abstraction teaching model (complements `proper/S6-driver`, `improper-17-bsp`, and `hal/polyhal` in the material).
- **nanos-lite**: a小 teaching OS running on AM; **navy-apps**: user programs.

**Related emulator/DiffTest references (golden oracles):**
- **QEMU source**: `~/temp/qemu-camp-2026-exper-Lfan-ke` (also `~/tgln/stage2/material/hyper/qemu`) — industrial full-system emulator / TCG / virtio. For `proper/S18-tcg`, `improper/19-isa-emulator`, `proper/S17-virt`.
- **Spike source**: under `~/ysyx/ysyx-workbench/nemu/` (NEMU's `tools/spike-diff` uses it) — official RISC-V ISA simulator, the trusted DiffTest oracle. Teaching labs use a self-contained golden trace; 引申 = swap in real Spike/QEMU as the per-instruction oracle.

Use for the standing goal「完善完成 OSLAB」: add experiments / angles (AM-style HAL, NEMU-style emulator + DiffTest, nanos-lite OS design). See also [[material-reference-tgln-stage2]] (primary全栈 reference) — look there first too.
