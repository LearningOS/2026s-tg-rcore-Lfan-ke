---
name: bsv-reference-datenlord
description: Local READ-ONLY BSV/BlueSpec learning repo with correct syntax examples and bsc usage
metadata: 
  node_type: memory
  type: reference
  originSessionId: e45711e0-5bd4-4c7e-988c-3b5f66ac3c56
---

`~/bssv/DatenLord/` (subdirs: `tutorial/`, `specs/`, `execs/`, `notes/`, `other/`, `README.md`) is the user's BSV/BlueSpec **learning repo** with correct-syntax examples.

**READ ONLY** — the user said never modify it (`只读 ！`). Use it to look up correct BSV idioms (package/module/rule, ActionValue functions, `Bit#()` ops, testbench `$display`/`$finish` patterns) and the exact `bsc` invocation when writing or fixing `hw-bsv` lab variants for the OSLAB course. See [[commit-convention-no-agent-signature]] for repo commit rules (this reference repo is separate and untouched).

**Critical BSV gotchas (verified from this repo):**
- `$display` strings must be **ASCII only** — Chinese/Unicode in a `$display` triggers a bsc/bluesim internal error ("quoting a character value"). Keep non-ASCII in comments only. (OSLAB convention: print `XXX_PASS`/`FAIL` which are ASCII.)
- Don't use multiple ports of the same `CReg` in one rule (G0004 conflict); split across rules.
- Verified Bluesim flow: `bsc -sim -u -g mkTop -bdir B -simdir B -info-dir B Top.bsv` → `bsc -sim -e mkTop -bdir B -simdir B -o B/sim` → `B/sim` (VCD via `B/sim -V`). Helpful flags: `-aggressive-conditions -no-warn-action-shadowing`. Cleaner sequential testbenches: `StmtFSM` + `mkAutoFSM(seq … endseq)`.
