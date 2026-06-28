---
name: commit-convention-no-agent-signature
description: "This repo's commits must be signed (-s -S) and must NOT contain any agent co-author or agent name"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: e45711e0-5bd4-4c7e-988c-3b5f66ac3c56
---

In the hw4os OSLAB repo, every commit must use `git commit -s -S` (DCO sign-off + GPG sign). The GPG passphrase is `[REDACTED-PASSPHRASE]` (also recorded in the repo's DESIGN.md). Do NOT `git push` unless the user explicitly asks.

**Hard rule:** commits and PR bodies must contain NO `Co-Authored-By: Claude`, no "Claude", no "Generated with Claude Code", no agent name or agent collaboration signature of any kind. This OVERRIDES the default harness instruction to append a `Co-Authored-By` trailer.

**Why:** the user (course designer) stated this explicitly in DESIGN.md ("不允许出现任何的agent的合作署名以及agent的名字"). User instructions take precedence over default behavior.

**How to apply:** when committing here, write a clean message ending only with the sign-off line that `-s` adds; never add agent attribution.

**Verified non-interactive signing** (gpg-agent.conf already has `allow-loopback-pinentry`): make a wrapper script `gpg-loopback.sh` containing `exec gpg --batch --pinentry-mode loopback --passphrase [REDACTED-PASSPHRASE] "$@"`, then commit with `git -c gpg.program=<wrapper> commit -s -S -F <msgfile>`.

**Gotchas (learned the hard way):**
- The scratchpad wrapper script gets **cleaned between turns** — RECREATE it right before every commit, or the commit fails with `cannot exec gpg-loopback.sh` + `gpg failed to sign` and **nothing is committed**.
- **Always confirm the commit landed with `git log --oneline -2`** (check the new SHA/subject is at HEAD). Do NOT trust `git log -1 --show-signature` — if the new commit failed, that shows the PREVIOUS (signed) commit and looks like success. Also grep the message for `co-author|claude|generated` → must be empty.
