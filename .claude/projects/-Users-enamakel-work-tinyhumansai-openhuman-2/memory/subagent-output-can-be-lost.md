---
name: subagent-output-can-be-lost
description: Long-running subagents can die/roll back losing uncommitted work; commit verified subagent output promptly
metadata:
  type: feedback
---

During the agent_workflows feature build, a `codecrusher` subagent ran ~20 min, hit an API socket error, and its working-tree output was largely rolled back (only 2 of ~10 files survived) — and a separate harness subagent got stuck in a loop re-running 5-minute background `cargo test` commands without converging, requiring a manual TaskStop + `pkill -f "cargo test"`.

**How to apply:**
- When a subagent completes a self-contained, independently-verified slice (e.g. frontend in `app/` only), **commit it promptly** as a checkpoint rather than letting it sit uncommitted while other agents run — uncommitted work is the only thing at risk of a rollback.
- Give parallel subagents **non-overlapping path scopes** (one in `src/`, one in `app/`) and tell them NOT to commit, so the main thread reconciles and commits.
- If a subagent goes quiet, check liveness via file mtimes + `TaskOutput(block:false)`; if it's looping on long background commands, `TaskStop` it and `pkill` stray `cargo` processes, then finish the work directly.
- Always independently re-verify a subagent's claimed results — see [[cargo-check-vs-test-verification]] (a subagent reported "tests pass" when the test cfg never compiled).
